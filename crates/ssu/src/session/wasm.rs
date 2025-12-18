use wasm_bindgen::{JsValue, prelude::wasm_bindgen};

use crate::session::{SessionEndpoint, SessionRecvEndpoint, SessionSendEndpoint, Ticked};
use std::io;

#[wasm_bindgen]
extern "C" {}

pub struct WasmSession {
    read_fn: js_sys::Function,
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

impl WasmSession {
    pub fn new(read_fn: String, write_fn: String) -> io::Result<WasmSession> {
        let read_fn = js_sys::Function::from(js_sys::eval(&read_fn).map_err(to_io_error)?);
        if !read_fn.is_function() {
            return Err(io::Error::other("read_fn was not a function"));
        }
        let write_fn = js_sys::Function::from(js_sys::eval(&write_fn).map_err(to_io_error)?);
        if !write_fn.is_function() {
            return Err(io::Error::other("write_fn was not a function"));
        }
        Ok(Self { read_fn, write_fn })
    }

    pub fn new_message_channel() -> io::Result<WasmSession> {
        let array = js_sys::eval(
            r#"
        (function () {
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
                for (let i = 0; i < data.length; i++) {
                    readQueue.push(data[i]);
                }
            };

            window.onmessage = function (event) {
                delete window.onmessage;
                clearInterval(interval);
                window.parent.postMessage({ type: "open" }, "*", [messageChannel.port2]);
                port.postMessage({ type: "read" });
            };

            function js_read(data) {
                if (readQueue.length === 0 && !reading) {
                    reading = true;
                    port.postMessage({ type: "read" });
                }
                return readQueue.shift();
            }
            function js_write(data) {
                port.postMessage({ type: "write", data });
            }
            return [js_read, js_write];
        })();
        "#,
        )
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
        Ok(Self { read_fn, write_fn })
    }
}

impl SessionEndpoint for WasmSession {
    fn recv(&mut self) -> Ticked {
        let b = self.read_fn.call0(&JsValue::UNDEFINED);
        let Ok(b) = b else {
            return Ticked::Idle;
        };
        if b.is_null_or_undefined() {
            return Ticked::Idle;
        }
        Ticked::Byte(b.as_f64().unwrap_or_default() as u8)
    }

    fn send(&mut self, b: u8) {
        self.write_fn
            .call1(&JsValue::UNDEFINED, &JsValue::from(b as f64));
    }

    fn split(
        self: Box<Self>,
    ) -> (
        Box<dyn SessionRecvEndpoint + Send + 'static>,
        Box<dyn SessionSendEndpoint + Send + 'static>,
    ) {
        unimplemented!()
    }
}
