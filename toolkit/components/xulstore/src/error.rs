/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

use nserror::{nsresult, NS_ERROR_FAILURE, NS_ERROR_NOT_AVAILABLE, NS_ERROR_UNEXPECTED, NS_OK};
use nsstring::nsCString;
use rkv::StoreError as RkvStoreError;
use serde_json::Error as SerdeJsonError;
use std::{ffi::NulError, io::Error as IoError, str::Utf8Error, string::FromUtf16Error, sync::PoisonError};

pub(crate) type XULStoreResult<T> = Result<T, XULStoreError>;

// This newtype enables us to implement From<XULStoreResult> for nsresult.
pub struct XULStoreNsResult(pub nsresult);

#[derive(Debug, Fail)]
pub enum XULStoreError {
    #[fail(display = "error converting bytes: {:?}", _0)]
    ConvertBytes(Utf8Error),

    #[fail(display = "error converting string: {:?}", _0)]
    ConvertString(FromUtf16Error),

    #[fail(display = "I/O error: {:?}", _0)]
    IoError(IoError),

    #[fail(display = "iteration is finished")]
    IterationFinished,

    #[fail(display = "JSON error: {}", _0)]
    JsonError(SerdeJsonError),

    // This should never happen, since we only pass string literals
    // to ffi::CString::new().  Nevertheless, we have to account for it,
    // since that constructor returns a Result.
    #[fail(display = "interior nul byte")]
    NulError,

    // NB: We can avoid storing the nsCString error description
    // once nsresult is a real type with a Display implementation
    // per https://bugzilla.mozilla.org/show_bug.cgi?id=1513350.
    #[fail(display = "error result {}", _0)]
    Nsresult(nsCString, nsresult),

    #[fail(display = "poison error getting read/write lock")]
    PoisonError,

    #[fail(display = "store error: {:?}", _0)]
    RkvStoreError(RkvStoreError),

    #[fail(display = "unavailable")]
    Unavailable,

    #[fail(display = "unexpected value")]
    UnexpectedValue,
}

impl From<XULStoreError> for nsresult {
    fn from(err: XULStoreError) -> nsresult {
        match err {
            XULStoreError::ConvertBytes(_) => NS_ERROR_FAILURE,
            XULStoreError::ConvertString(_) => NS_ERROR_FAILURE,
            XULStoreError::IoError(_) => NS_ERROR_FAILURE,
            XULStoreError::IterationFinished => NS_ERROR_FAILURE,
            XULStoreError::JsonError(_) => NS_ERROR_FAILURE,
            XULStoreError::Nsresult(_, result) => result,
            XULStoreError::NulError => NS_ERROR_UNEXPECTED,
            XULStoreError::PoisonError => NS_ERROR_UNEXPECTED,
            XULStoreError::RkvStoreError(_) => NS_ERROR_FAILURE,
            XULStoreError::Unavailable => NS_ERROR_NOT_AVAILABLE,
            XULStoreError::UnexpectedValue => NS_ERROR_UNEXPECTED,
        }
    }
}

impl<T> From<XULStoreResult<T>> for XULStoreNsResult {
    fn from(result: XULStoreResult<T>) -> XULStoreNsResult {
        match result {
            Ok(_) => XULStoreNsResult(NS_OK),
            Err(err) => XULStoreNsResult(err.into()),
        }
    }
}

impl From<FromUtf16Error> for XULStoreError {
    fn from(err: FromUtf16Error) -> XULStoreError {
        XULStoreError::ConvertString(err)
    }
}

impl From<nsresult> for XULStoreError {
    fn from(result: nsresult) -> XULStoreError {
        XULStoreError::Nsresult(result.error_name(), result)
    }
}

impl<T> From<PoisonError<T>> for XULStoreError {
    fn from(_: PoisonError<T>) -> XULStoreError {
        XULStoreError::PoisonError
    }
}

impl From<RkvStoreError> for XULStoreError {
    fn from(err: RkvStoreError) -> XULStoreError {
        XULStoreError::RkvStoreError(err)
    }
}

impl From<Utf8Error> for XULStoreError {
    fn from(err: Utf8Error) -> XULStoreError {
        XULStoreError::ConvertBytes(err)
    }
}

impl From<IoError> for XULStoreError {
    fn from(err: IoError) -> XULStoreError {
        XULStoreError::IoError(err)
    }
}

impl From<NulError> for XULStoreError {
    fn from(_: NulError) -> XULStoreError {
        XULStoreError::NulError
    }
}

impl From<SerdeJsonError> for XULStoreError {
    fn from(err: SerdeJsonError) -> XULStoreError {
        XULStoreError::JsonError(err)
    }
}
