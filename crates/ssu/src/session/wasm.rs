use atomic_waker::AtomicWaker;
use wasm_bindgen::prelude::Closure;
use wasm_bindgen::{JsValue, prelude::wasm_bindgen};

use crate::session::{SessionPartsUnsend, SessionRecvEndpoint, SessionSendEndpoint, SessionUnsend};
use std::sync::Arc;
use std::task::{Context, Poll};
use std::{fmt, io};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WasmConfig {
    pub read_fn: String,
    pub write_fn: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageChannelConfig {}

#[wasm_bindgen]
extern "C" {}

pub struct WasmRecvSession {
    read_fn: js_sys::Function,
    read_buffer: js_sys::Uint8Array,
    read_buffer_index: u32,
    waker: Arc<AtomicWaker>,
}

pub struct WasmSendSession {
    write_fn: js_sys::Function,
}

fn to_io_error(e: impl AsRef<wasm_bindgen::JsValue>) -> io::Error {
    use js_sys::Object;
    if let Some(e) = e.as_ref().as_string() {
        return io::Error::other(e);
    }
    if let Some(obj) = Object::try_from(e.as_ref()) {
        return io::Error::other(obj.to_string().as_string().unwrap_or_default());
    }
    if let Ok(s) = js_sys::JSON::stringify(e.as_ref()) {
        return io::Error::other(s.as_string().unwrap_or_default());
    }
    io::Error::other("Unknown error")
}

impl SessionUnsend for WasmConfig {
    fn boot(self) -> std::io::Result<SessionPartsUnsend> {
        let waker = Arc::new(AtomicWaker::new());
        let waker_clone = waker.clone();
        let got_read = Closure::wrap(Box::new(move || {
            waker_clone.wake();
        }) as Box<dyn FnMut()>);

        // Convert the async function into one that returns null if a promise is resolving.
        let read_fn = js_sys::Function::from(
            js_sys::Function::new_with_args(
                "got_read",
                &r#"
            const LOW_WATER_MARK = 2;
            const async_fn = __FN__;
            let promise = undefined;
            let next = [];
            let then = (value) => {
                next.push(value);
                if (next.length < LOW_WATER_MARK && promise === undefined) {
                    promise = async_fn().then(then);
                } else {
                    promise = undefined;
                }
                got_read();
            };

            return function() {
                if (next.length === 0) {
                    if (promise === undefined) {
                        promise = async_fn().then(then);
                    }
                    return null;
                }
                const result = next.shift();
                if (next.length < LOW_WATER_MARK && promise === undefined) {
                    promise = async_fn().then(then);
                }
                return result;
            };
        "#
                .replace("__FN__", &self.read_fn),
            )
            .call1(&JsValue::UNDEFINED, &got_read.into_js_value())
            .map_err(to_io_error)?,
        );
        if !read_fn.is_function() {
            return Err(io::Error::other("read_fn was not a function"));
        }
        let write_fn = js_sys::Function::from(js_sys::eval(&self.write_fn).map_err(to_io_error)?);
        if !write_fn.is_function() {
            return Err(io::Error::other("write_fn was not a function"));
        }
        Ok(SessionPartsUnsend::new(
            WasmSendSession { write_fn },
            WasmRecvSession {
                read_fn,
                read_buffer: js_sys::Uint8Array::new_with_length(0),
                read_buffer_index: 0,
                waker,
            },
        ))
    }
}

impl SessionUnsend for MessageChannelConfig {
    fn boot(self) -> std::io::Result<SessionPartsUnsend> {
        let waker = Arc::new(AtomicWaker::new());
        let waker_clone = waker.clone();
        let got_read = Closure::wrap(Box::new(move || {
            waker_clone.wake();
        }) as Box<dyn FnMut()>);

        let array = js_sys::Function::new_with_args(
            "got_read",
            r#"
            const LOW_WATER_MARK = 2;
            let messageChannel = new MessageChannel();
            const interval = setInterval(function () {
                window.parent.postMessage({ type: "ready" }, "*");
            }, 250);

            var reading = false;
            const readQueue = [];
            const port = messageChannel.port1;
            port.onmessage = function (event) {
                reading = false;
                const data = new Uint8Array(event.data.data);
                readQueue.push(data);
                got_read();
            };

            window.onmessage = function (event) {
                delete window.onmessage;
                clearInterval(interval);
                window.parent.postMessage({ type: "open" }, "*", [messageChannel.port2]);
                port.postMessage({ type: "read" });
            };

            function js_read(data) {
                if (readQueue.length < LOW_WATER_MARK && !reading) {
                    reading = true;
                    port.postMessage({ type: "read" });
                }
                return readQueue.shift();
            }
            function js_write(data) {
                port.postMessage({ type: "write", data });
            }
            return [js_read, js_write];
        "#,
        )
        .call1(&JsValue::UNDEFINED, &got_read.into_js_value())
        .map_err(to_io_error)?;

        let array = js_sys::Array::from(&array);
        let read_fn = js_sys::Function::from(array.get(0));
        if !read_fn.is_function() {
            return Err(io::Error::other("read_fn was not a function"));
        }
        let write_fn = js_sys::Function::from(array.get(1));
        if !write_fn.is_function() {
            return Err(io::Error::other("write_fn was not a function"));
        }

        Ok(SessionPartsUnsend::new(
            WasmSendSession { write_fn },
            WasmRecvSession {
                read_fn,
                read_buffer: js_sys::Uint8Array::new_with_length(0),
                read_buffer_index: 0,
                waker,
            },
        ))
    }
}

impl SessionRecvEndpoint for WasmRecvSession {
    fn poll_recv(&mut self, ctx: &mut Context<'_>) -> Poll<io::Result<u8>> {
        if self.read_buffer_index < self.read_buffer.byte_length() as _ {
            self.read_buffer_index += 1;
            return Poll::Ready(Ok(self.read_buffer.get_index(self.read_buffer_index - 1)));
        }
        let b = self.read_fn.call0(&JsValue::UNDEFINED);
        let Ok(b) = b else {
            self.waker.register(ctx.waker());
            return Poll::Pending;
        };
        if b.is_null_or_undefined() {
            self.waker.register(ctx.waker());
            return Poll::Pending;
        }
        self.read_buffer = js_sys::Uint8Array::from(b);
        self.read_buffer_index = 1;
        Poll::Ready(Ok(self.read_buffer.get_index(0)))
    }
}

impl SessionSendEndpoint for WasmSendSession {
    fn poll_send(&mut self, _ctx: &mut Context<'_>, b: u8) -> std::task::Poll<io::Result<()>> {
        let b = self
            .write_fn
            .call1(&JsValue::UNDEFINED, &JsValue::from(b as f64));
        let Ok(_) = b else {
            return Poll::Ready(Err(io::Error::other("Failed to write byte")));
        };
        Poll::Ready(Ok(()))
    }
}

impl fmt::Debug for WasmRecvSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "WasmRecvSession")
    }
}

impl fmt::Debug for WasmSendSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "WasmSendSession")
    }
}
