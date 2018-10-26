/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

#![allow(non_snake_case)]

extern crate xpcom;

use nserror::{nsresult, NS_ERROR_FAILURE, NS_OK};
use nsstring::{nsACString, nsCString};
use std::{cell::Cell, fmt::Write, ptr, result};
use xpcom::{
    getter_addrefs,
    interfaces::{nsIKeyValueDatabase, nsIRunnable, nsIThread},
    RefPtr,
};

pub type Result<T> = result::Result<T, nsresult>;

extern "C" {
    fn NS_GetCurrentThreadEventTarget(result: *mut *const nsIThread) -> nsresult;
    fn NS_NewNamedThreadWithDefaultStackSize(
        name: *const nsACString,
        result: *mut *const nsIThread,
        event: *const nsIRunnable,
    ) -> nsresult;
}

/// Returns a handle to the current thread.
pub fn get_current_thread() -> Result<RefPtr<nsIThread>> {
    getter_addrefs(|p| unsafe { NS_GetCurrentThreadEventTarget(p) })
}

pub fn create_thread(name: &str) -> Result<RefPtr<nsIThread>> {
    let name: nsCString = name.into();
    getter_addrefs(|p| unsafe { NS_NewNamedThreadWithDefaultStackSize(&*name, p, ptr::null()) })
}

/// A task is executed asynchronously on a target thread, and passes its
/// result back to the original thread.
pub trait Task {
    fn run(&self) -> Result<RefPtr<nsIKeyValueDatabase>>;
    fn done(&self, result: Result<RefPtr<nsIKeyValueDatabase>>) -> nsresult;
}

#[derive(xpcom)]
#[xpimplements(nsIRunnable, nsINamed)]
#[refcnt = "atomic"]
pub struct InitTaskRunnable {
    name: &'static str,
    source: RefPtr<nsIThread>,

    /// Holds the task, and the result of the task. The task is created on the
    /// current thread, run on a target thread, and handled again on the
    /// original thread; the result is mutated on the target thread and
    /// accessed on the original thread.
    task: Box<Task>,
    result: Cell<Option<Result<RefPtr<nsIKeyValueDatabase>>>>,
}

impl TaskRunnable {
    pub fn new(
        name: &'static str,
        source: RefPtr<nsIThread>,
        task: Box<Task>,
        result: Cell<Option<Result<RefPtr<nsIKeyValueDatabase>>>>,
    ) -> RefPtr<TaskRunnable> {
        TaskRunnable::allocate(InitTaskRunnable {
            name,
            source,
            task,
            result,
        })
    }

    unsafe fn Run(&self) -> nsresult {
        match self.result.take() {
            None => {
                // Run the task on the storage thread, store the result, and
                // dispatch the runnable back to the source thread.
                let result = self.task.run();
                self.result.set(Some(result));
                let target = getter_addrefs(|p| self.source.GetEventTarget(p)).unwrap();
                target.DispatchFromScript(self.coerce(), 0)
            }
            Some(result) => {
                // Back on the source thread, notify the task we're done.
                self.task.done(result)
            }
        }
    }

    unsafe fn GetName(&self, name: *mut nsACString) -> nsresult {
        match write!(*name, "{}", self.name) {
            Ok(()) => NS_OK,
            Err(_) => NS_ERROR_FAILURE,
        }
    }
}
