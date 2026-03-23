#![crate_type = "cdylib"]

use js_sys::{Function, Uint8Array};
use std::collections::BTreeMap;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct DecoderRegistry {
    data: Vec<u8>,
    decoders: BTreeMap<u16, JsDecoder>,
}

#[wasm_bindgen]
impl DecoderRegistry {
    #[wasm_bindgen(constructor)]
    pub fn new() -> DecoderRegistry {
        DecoderRegistry {
            data: Vec::new(),
            decoders: BTreeMap::new(),
        }
    }

    #[wasm_bindgen]
    pub fn decode(&mut self, compression: u16) {
        self.decoders[&compression].decode(&mut self.data)
    }

    pub fn register_decoder(&mut self, compression: u16, decoder: JsDecoder) {
        self.decoders.insert(compression, decoder);
    }
}
#[wasm_bindgen]
pub struct JsDecoder {
    func: Function,
}

#[wasm_bindgen]
impl JsDecoder {
    #[wasm_bindgen(constructor)]
    pub fn new(func: Function) -> JsDecoder {
        JsDecoder { func }
    }

    pub fn decode(&self, data: &mut [u8]) {
        let view = unsafe { Uint8Array::view_mut_raw(data.as_mut_ptr(), data.len()) };
        self.func.call1(&JsValue::NULL, &view).unwrap();
    }
}
