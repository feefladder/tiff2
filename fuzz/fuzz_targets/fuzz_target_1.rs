#![no_main]

use libfuzzer_sys::fuzz_target;
use tokio::runtime::Runtime;

fuzz_target!(|data: &[u8]| {
    let rt = Runtime::new().unwrap();
    rt.block_on(
    async {
        let mut decoder = if let Ok(d) = tiff2::decoder::Decoder::new(data).await {
            d
        } else {
            return;
        };

        decoder.scan_ifds().await.unwrap();
        decoder.read_image_ifds().await.unwrap();
    }
    );
});
