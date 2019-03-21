/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

use crate::{error::XULStoreError, error::XULStoreResult, ffi::ProfileChangeObserver, make_key, SEPARATOR};
use nsstring::nsString;
use rkv::{Manager, Rkv, SingleStore, StoreOptions, Value};
use std::{
    collections::HashMap,
    ffi::CString,
    fs::{create_dir_all, remove_file, File},
    ops::DerefMut,
    path::PathBuf,
    str,
    sync::{Arc, RwLock},
};
use xpcom::{interfaces::nsIFile, XpCom};

type XULStoreData = HashMap<String, HashMap<String, HashMap<String, String>>>;

lazy_static! {
    pub(crate) static ref PROFILE_DIR: RwLock<Option<PathBuf>> = {
        observe_profile_change();
        RwLock::new(get_profile_dir().ok())
    };

    #[derive(Debug)]
    pub(crate) static ref RKV: RwLock<Option<Arc<RwLock<Rkv>>>> = {
        RwLock::new(get_rkv().ok())
    };

    #[derive(Debug)]
    pub(crate) static ref STORE: RwLock<Option<SingleStore>> = {
        RwLock::new(match get_store() {
            Ok(store) => {
                maybe_migrate_data(store);
                Some(store)
            }
            Err(err) => {
                error!("error getting store: {}", err);
                None
            },
        })
    };

    pub(crate) static ref DATA: RwLock<Option<XULStoreData>> = {
        RwLock::new(get_data().ok())
    };
}

// Memoized to the PROFILE_DIR lazy static. Prefer that accessor to calling
// this function, to avoid extra trips across the XPCOM FFI.
//
// NB: this code must be kept in sync with the code that updates the store's
// location in toolkit/components/xulstore/XULStore.jsm.
pub(crate) fn get_profile_dir() -> XULStoreResult<PathBuf> {
    let dir_svc = xpcom::services::get_DirectoryService().ok_or(XULStoreError::Unavailable)?;
    let mut profile_dir = xpcom::GetterAddrefs::<nsIFile>::new();
    let property = CString::new("ProfD")?;
    unsafe {
        dir_svc.Get(property.as_ptr(), &nsIFile::IID, profile_dir.void_ptr());
    }
    let profile_dir = profile_dir.refptr().ok_or(XULStoreError::Unavailable)?;

    let mut profile_path = nsString::new();
    unsafe {
        profile_dir.GetPath(profile_path.deref_mut());
    }

    let path = String::from_utf16(&profile_path[..])?;
    Ok(PathBuf::from(&path))
}

fn get_xulstore_dir() -> XULStoreResult<PathBuf> {
    let mut xulstore_dir = PROFILE_DIR
        .read()?
        .as_ref()
        .ok_or(XULStoreError::Unavailable)?
        .clone();
    xulstore_dir.push("xulstore");
    info!("get XULStore dir: {:?}", &xulstore_dir);

    create_dir_all(xulstore_dir.clone())?;

    Ok(xulstore_dir)
}

pub(crate) fn get_rkv() -> XULStoreResult<Arc<RwLock<Rkv>>> {
    let mut manager = Manager::singleton().write()?;
    let xulstore_dir = get_xulstore_dir()?;
    manager
        .get_or_create(xulstore_dir.as_path(), Rkv::new)
        .map_err(|err| err.into())
}

pub(crate) fn get_store() -> XULStoreResult<SingleStore> {
    let rkv_guard = RKV.read()?;
    let rkv = rkv_guard
        .as_ref()
        .ok_or(XULStoreError::Unavailable)?
        .read()?;
    rkv.open_single("db", StoreOptions::create())
        .map_err(|err| err.into())
}

fn maybe_migrate_data(store: SingleStore) {
    // Failure to migrate data isn't fatal, so we don't return a result.
    // But we use a closure returning a result to enable use of the ? operator.
    (|| -> XULStoreResult<()> {
        let mut old_datastore = PROFILE_DIR
            .read()?
            .as_ref()
            .ok_or(XULStoreError::Unavailable)?
            .clone();
        old_datastore.push("xulstore.json");
        if !old_datastore.exists() {
            debug!("old datastore doesn't exist: {:?}", old_datastore);
            return Ok(());
        }

        let file = File::open(old_datastore.clone())?;
        let json: HashMap<String, HashMap<String, HashMap<String, String>>> =
            serde_json::from_reader(file)?;

        let rkv_guard = RKV.read()?;
        let rkv = rkv_guard
            .as_ref()
            .ok_or(XULStoreError::Unavailable)?
            .read()?;
        let mut writer = rkv.write()?;

        for (doc, ids) in json {
            for (id, attrs) in ids {
                for (attr, value) in attrs {
                    let key = make_key(&doc, &id, &attr);
                    store.put(&mut writer, &key, &Value::Str(&value))?;
                }
            }
        }

        writer.commit()?;

        remove_file(old_datastore)?;

        Ok(())
    })()
    .unwrap_or_else(|err| error!("error migrating data: {}", err));
}

fn observe_profile_change() {
    // Failure to observe the change isn't fatal (although it means we won't
    // persist XULStore data for this session), so we don't return a result.
    // But we use a closure returning a result to enable use of the ? operator.
    (|| -> XULStoreResult<()> {
        // Observe profile changes so we can update this directory accordingly.
        let obs_svc = xpcom::services::get_ObserverService().ok_or(XULStoreError::Unavailable)?;
        let observer = ProfileChangeObserver::new();
        let topic = CString::new("profile-after-change")?;
        unsafe {
            obs_svc
                .AddObserver(observer.coerce(), topic.as_ptr(), false)
                .to_result()?
        };
        Ok(())
    })()
    .unwrap_or_else(|err| error!("error observing profile change: {}", err));
}

pub(crate) fn update_profile_dir() {
    // Failure to update the dir isn't fatal (although it means that we won't
    // persist XULStore data for this session), so we don't return a result.
    // But we use a closure returning a result to enable use of the ? operator.
    (|| -> XULStoreResult<()> {
        {
            let mut profile_dir_guard = PROFILE_DIR.write()?;
            *profile_dir_guard = get_profile_dir().ok();
        }

        {
            // The get_store() call below will also try to lock the RKV static,
            // so we do this in a block to ensure the RwLockWriteGuard goes out
            // of scope and gets dropped (releasing the lock) beforehand.
            let mut rkv_guard = RKV.write()?;
            *rkv_guard = get_rkv().ok();
        }

        {
            let mut store_guard = STORE.write()?;
            *store_guard = match get_store() {
                Ok(store) => {
                    maybe_migrate_data(store);
                    Some(store)
                }
                Err(err) => {
                    error!("error getting store: {}", err);
                    None
                }
            };
        }

        let mut data_guard = DATA.write()?;
        *data_guard = get_data().ok();

        Ok(())
    })()
    .unwrap_or_else(|err| error!("error updating profile dir: {}", err));
}

fn unwrap_value(value: &Option<Value>) -> XULStoreResult<String> {
    match value {
        Some(Value::Str(val)) => Ok(val.to_string()),

        // Per the XULStore API, return an empty string if the value
        // isn't found.
        None => Ok("".to_owned()),

        // This should never happen, but it could happen in theory
        // if someone writes a different kind of value into the store
        // using a more general API (kvstore, rkv, LMDB).
        Some(_) => Err(XULStoreError::UnexpectedValue),
    }
}

fn get_data() -> XULStoreResult<XULStoreData> {
    let store = *STORE.read()?.as_ref().ok_or(XULStoreError::Unavailable)?;
    let rkv_guard = RKV.read()?;
    let rkv = rkv_guard
        .as_ref()
        .ok_or(XULStoreError::Unavailable)?
        .read()?;
    let reader = rkv.read()?;
    let mut all = HashMap::new();
    let iterator = store.iter_start(&reader)?;

    for result in iterator {
        let (key, value): (&str, String) = match result {
            Ok((key, value)) => {
                assert!(value.is_some(), "iterated key has value");
                match (str::from_utf8(&key), unwrap_value(&value)) {
                    (Ok(key), Ok(value)) => (key, value),
                    (Err(err), _) => return Err(err.into()),
                    (_, Err(err)) => return Err(err.into()),
                }
            }
            Err(err) => return Err(err.into()),
        };

        let parts = key.split(SEPARATOR).collect::<Vec<&str>>();
        if parts.len() != 3 {
            // TODO: make this UnexpectedKey.
            return Err(XULStoreError::UnexpectedValue);
        }
        let (doc_url, element_id, attr_name) = (parts[0].to_owned(), parts[1].to_owned(), parts[2].to_owned());

        let doc = all.entry(doc_url).or_insert(HashMap::new());
        let id = doc.entry(element_id).or_insert(HashMap::new());
        id.entry(attr_name).or_insert(value);
    }

    Ok(all)
}
