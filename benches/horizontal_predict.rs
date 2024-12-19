use criterion::{black_box, criterion_group, criterion_main, Criterion};
use rand::{Rng, SeedableRng};
///
/// [1 2 3 4 5 6 7 8] n_samp = 2
/// [1 2]| |
///   \--2 |          i=0; i=-2%2=0; in[i].wrapping_sub(prev[i%n_samp])
///   [3 2]|          prev[i%n_samp]=in[i]
///      \-2          i=1; i-n_samp%n_samp=-1%2=1;
/// this seems horribly memory inefficient...
fn hdiff_ssamp_buf(input: &mut [u8], n_samp: usize) {
    let mut prev = input[..n_samp].to_vec();
    for i in n_samp..input.len() {
        let p = input[i];
        input[i] = input[i].wrapping_sub(prev[i % n_samp]);
        prev[i % n_samp] = p
    }
}

/// maybe with a double-sized buffer...
/// [0 1 2 3 4 5 6 7] n_samp = 2; let d_samp = n_samp*2=4;
/// [0 1 2 3]         prev = in[..dsamp].clone();
///   \-[2]3 4 5 6 7  i=0; in[i]=2; in[i] = in[i].wrapping_sub(prev[i%dsamp])
/// [4 1 2 3]         prev[(i+n_samp) % dsamp] = in[i+n_samp]
///     \2[2]4 5 6 7  i=1; in[i] = 3-1 = 2
/// [4(5)2 3]         prev[1%4]=prev[1]=in[1+2]=in[3]=3
///      2\2[2]5 6 7  i=2; in[i]=  4-2 = 2
/// [4 5 6 3]         prev[2%4]=prev[2]=in[2+2]=in[4]=4
///      2 2\2[2]6 7  i=3; in[i]=  5-3 = 2
///         [4 5 6 7] prev[3%4]=prev[3]=in[3+2]=in[5]=5
///      -------------remainder
///      2 2 2 2[2]7
///      2 2 2 2 2[2]
fn hdiff_dsamp_buf(input: &mut [u8], n_samp: usize) {
    let d_samp = 2 * n_samp;
    let mut prev = input[..d_samp].to_vec();
    for i in n_samp..input.len() - n_samp {
        input[i] = input[i].wrapping_sub(prev[i % d_samp]);
        prev[(i + n_samp) % d_samp] = input[i + n_samp];
    }
    for i in input.len() - n_samp..input.len() {
        input[i] = input[i].wrapping_sub(prev[i % d_samp]);
    }
}
/// stolen from image-tiff
fn hdiff_copy(input: &[u8], n_samp: usize) -> Vec<u8> {
    let mut res = Vec::with_capacity(input.len());
    let (start, rest) = input.split_at(n_samp);

    res.extend_from_slice(start);
    if res.capacity() - res.len() < rest.len() {
        return res;
    }

    res.extend(
        input
            .into_iter()
            .zip(rest)
            .map(|(prev, current)| current.wrapping_sub(*prev)),
    );
    res
}

fn benchmark_horizontal_difference(c: &mut Criterion) {
    // Prepare input data
    let sample_sizes = [1, 2, 4, 8]; // Adjust based on your needs
    let input_sizes = [100_000, 500_000, 1_000_000]; // Size of the input array

    for input_size in input_sizes {
        for sample_size in sample_sizes {
            let mut rng = rand::rngs::StdRng::seed_from_u64(input_size + sample_size);
            let data: Vec<u8> = (0..input_size).map(|_| rng.gen()).collect();
            // Run the benchmarks
            c.bench_function(&format!("hdiff_ssamp_buf{input_size}_{sample_size}"), |b| {
                b.iter(|| {
                    let mut input = data.clone(); // Clone for the copy-based method
                    hdiff_ssamp_buf(
                        black_box(&mut input),
                        black_box(usize::try_from(sample_size).unwrap()),
                    );
                })
            });

            c.bench_function(&format!("hdif_dsamp_buf{input_size}_{sample_size}"), |b| {
                b.iter(|| {
                    let mut input = data.clone(); // Clone for the no-copy method
                    hdiff_dsamp_buf(
                        black_box(&mut input),
                        black_box(usize::try_from(sample_size).unwrap()),
                    );
                })
            });

            c.bench_function(&format!("hdiff_copy{input_size}_{sample_size}"), |b| {
                b.iter(|| {
                    hdiff_copy(
                        black_box(&data),
                        black_box(usize::try_from(sample_size).unwrap()),
                    );
                })
            });
        }
    }
}

criterion_group!(benches, benchmark_horizontal_difference);
criterion_main!(benches);
