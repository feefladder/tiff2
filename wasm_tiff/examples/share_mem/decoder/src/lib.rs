use js_sys::Uint8Array;

#[wasm_bindgen]
pub struct DoubleDecoder;

#[wasm_bindgen]
impl DoubleDecoder {
    #[wasm_bindgen]
    pub fn decode(data: &mut Uint8Array) {
        for val in data {
            val *= 2
        }
    }
}
