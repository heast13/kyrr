//! The native engine's INT8 kernels for newer CPUs (AVX-VNNI, AVX-512 VNNI)
//! give exactly the output of the AVX2 ones. A file of its own: the kernel
//! choice is process-wide. Skipped without assets or a CPU for the engine.

use std::path::PathBuf;

use kyrr_denoise::{load, use_int8_kernel, Model, VoiceFilter};

#[test]
fn every_kernel_gives_the_same_output() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    if !kyrr_denoise::native_possible(&dir.join(Model::Light.file_name())) {
        eprintln!("assets or AVX2 missing, skipped");
        return;
    }
    // Speech-like: a gliding tone with noise, quiet stretches between.
    let rate = 48_000;
    let mut seed = 7u32;
    let input: Vec<f32> = (0..rate * 3)
        .map(|i| {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            let t = i as f32 / rate as f32;
            let voice = if (i / 9600) % 3 == 2 {
                0.0
            } else {
                0.3 * (t * (900.0 + 300.0 * t)).sin()
            };
            voice + (seed as f32 / u32::MAX as f32 - 0.5) * 0.05
        })
        .collect();
    for model in [Model::Light, Model::Quality] {
        let mut outputs = Vec::new();
        for kernel in ["avx2", "avx-vnni", "avx512-vnni"] {
            if !use_int8_kernel(kernel) {
                println!("{model:?}: no {kernel} on this CPU");
                continue;
            }
            let denoiser = load(&dir, model, None).expect("model loads");
            let mut filter = VoiceFilter::new(denoiser, rate as u32, 1, 480).expect("format accepted");
            let mut output = input.clone();
            for block in output.chunks_mut(480) {
                filter.process_in_place(block);
            }
            outputs.push((kernel, output));
        }
        let (_, avx2) = &outputs[0];
        for (kernel, output) in &outputs[1..] {
            let same = avx2.iter().zip(output).all(|(a, b)| a.to_bits() == b.to_bits());
            assert!(same, "{model:?}: {kernel} differs from avx2");
            println!("{model:?}: {kernel} gives the same output as avx2");
        }
    }
    use_int8_kernel("avx2");
}
