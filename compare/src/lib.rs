use std::path::PathBuf;

pub fn tiff_dir() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR")?).join("../tiffs");
    if !dir.is_dir() {
        std::fs::create_dir(&dir)?
    }
    Ok(dir)
}
