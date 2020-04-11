// Copyright © 2017 Mozilla Foundation
//
// This program is made available under an ISC-style license.  See the
// accompanying file LICENSE for details
#![warn(unused_extern_crates)]

#[macro_use]
extern crate error_chain;
#[macro_use]
extern crate log;

use audio_thread_priority::promote_current_thread_to_real_time;
use audioipc::core;
use audioipc::platformhandle_passing::framed_with_platformhandles;
use audioipc::rpc;
use audioipc::{MessageStream, PlatformHandle, PlatformHandleType};
use tokio::sync::oneshot;
use once_cell::sync::Lazy;
use std::ffi::{CStr, CString};
use std::os::raw::c_void;
use std::ptr;
use std::sync::Mutex;
use tokio::runtime;

mod server;

struct CubebContextParams {
    context_name: CString,
    backend_name: Option<CString>,
}

static G_CUBEB_CONTEXT_PARAMS: Lazy<Mutex<CubebContextParams>> = Lazy::new(|| {
    Mutex::new(CubebContextParams {
        context_name: CString::new("AudioIPC Server").unwrap(),
        backend_name: None,
    })
});

#[allow(deprecated)]
pub mod errors {
    error_chain! {
        links {
            AudioIPC(::audioipc::errors::Error, ::audioipc::errors::ErrorKind);
        }
        foreign_links {
            Cubeb(cubeb_core::Error);
            Io(::std::io::Error);
            Canceled(::futures::channel::oneshot::Canceled);
        }
    }
}

use crate::errors::*;

struct ServerWrapper {
    core_thread: core::CoreThread,
    callback_thread: core::CoreThread,
}

fn run() -> Result<ServerWrapper> {
    trace!("Starting up cubeb audio server event loop thread...");

    let callback_thread = core::spawn_thread(
        "AudioIPC Callback RPC",
        |_cx| {
            match promote_current_thread_to_real_time(0, 48000) {
                Ok(_) => {}
                Err(_) => {
                    debug!("Failed to promote audio callback thread to real-time.");
                }
            }
            trace!("Starting up cubeb audio callback event loop thread...");
            Ok(())
        },
        || {},
    )
    .or_else(|e| {
        debug!(
            "Failed to start cubeb audio callback event loop thread: {:?}",
            e
        );
        Err(e)
    })?;

    let core_thread =
        core::spawn_thread("AudioIPC Server RPC", move |_cx| Ok(()), || {}).or_else(|e| {
            debug!("Failed to cubeb audio core event loop thread: {:?}", e);
            Err(e)
        })?;

    Ok(ServerWrapper {
        core_thread,
        callback_thread,
    })
}

#[no_mangle]
pub unsafe extern "C" fn audioipc_server_start(
    context_name: *const std::os::raw::c_char,
    backend_name: *const std::os::raw::c_char,
) -> *mut c_void {
    let mut params = G_CUBEB_CONTEXT_PARAMS.lock().unwrap();
    if !context_name.is_null() {
        params.context_name = CStr::from_ptr(context_name).to_owned();
    }
    if !backend_name.is_null() {
        let backend_string = CStr::from_ptr(backend_name).to_owned();
        params.backend_name = Some(backend_string);
    }
    match run() {
        Ok(server) => Box::into_raw(Box::new(server)) as *mut _,
        Err(_) => ptr::null_mut() as *mut _,
    }
}

#[no_mangle]
pub extern "C" fn audioipc_server_new_client(p: *mut c_void) -> PlatformHandleType {
    let (wait_tx, wait_rx) = oneshot::channel();
    let wrapper: &ServerWrapper = unsafe { &*(p as *mut _) };

    let core_handle = wrapper.callback_thread.handle();

    // We create a connected pair of anonymous IPC endpoints. One side
    // is registered with the reactor core, the other side is returned
    // to the caller.
    MessageStream::anonymous_ipc_pair()
        .and_then(|(ipc_server, ipc_client)| {
            use futures_util::TryFutureExt;
            // Spawn closure to run on same thread as reactor::Core
            // via remote handle.
            wrapper
                .core_thread
                .handle()
                .spawn(futures::future::lazy(|_cx| {
                    trace!("Incoming connection");

                    let pool = futures::executor::LocalPool::new();
                    use futures_util::task::LocalSpawnExt;
                    pool.spawner().spawn_local_with_handle(
                        futures::future::lazy(|_| {
                            let handle = runtime::Handle::current();
                            ipc_server.into_tokio_ipc(&handle)
                        })
                        .and_then(|sock| {
                            let transport = framed_with_platformhandles(sock, Default::default());
                            rpc::bind_server(transport, server::CubebServer::new(core_handle))
                            .and_then(|_| async { wait_tx.send(()); Ok(()) } )
                        })
                    ).unwrap()
                    // Notify waiting thread that server has been registered.
                }));
            // Wait for notification that server has been registered
            // with reactor::Core.
            let _ = futures::executor::block_on(wait_rx);
            println!("Server seems OK");
            Ok(unsafe { PlatformHandle::from(ipc_client).into_raw() })
        })
        .unwrap_or(audioipc::INVALID_HANDLE_VALUE)
}

#[no_mangle]
pub extern "C" fn audioipc_server_stop(p: *mut c_void) {
    let wrapper = unsafe { Box::<ServerWrapper>::from_raw(p as *mut _) };
    drop(wrapper);
}
