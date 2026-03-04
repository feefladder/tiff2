use tiff2::error::{TiffError, TiffResult};

fn row_major_to_col_major<T>(
    data: &mut [Option<T>],
    width: usize,
    height: usize,
) -> TiffResult<Vec<T>> {
    assert_eq!(
        data.len(),
        width * height,
        "Data size must match dimensions"
    );

    let mut rearranged = Vec::with_capacity(data.len());

    for old_i in 0..data.len() {
        // for 5-wide,4-tall
        //  0  1  2  3  4
        //  5  6  7  8  9
        // 10 11 12 13 14
        // 15 16 17 18 19
        // new
        //  0  4  8 12 16
        //  1  5  9 13 17
        //  2  6 10 14 18
        //  3  7 11 15 19
        // old_i 0  1  2  3   4  5  6  7   8  9 10 11  12 13 14 15
        // new_i 0  5 10 15   1  6 11 16   2  7 12 17   3  8 13 18
        // (old_i%4)*5 + old_i/4
        // (old_i%height)*width + old_i/height
        rearranged.push(
            data[(old_i % height) * width + old_i / height]
                .take()
                .ok_or(TiffError::LimitsExceeded)?,
        )
    }

    Ok(rearranged)
}

fn main() -> TiffResult<()> {
    let height = 4;
    let width = 5;
    let mut data: Vec<Option<usize>> = (0..width * height).map(Some).collect(); // Example input: [0..19]

    println!("Original:");
    for chunk in data.chunks(width) {
        println!("{:?}", chunk);
    }

    let rearranged = row_major_to_col_major(&mut data, width, height)?;

    println!("\nRearranged:");
    for chunk in rearranged.chunks(height) {
        println!("{:?}", chunk);
    }
    Ok(())
}
