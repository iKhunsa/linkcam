//! `cargo run --release -p linkcam-vision --example blur -- <models-dir> in.jpg out.png [label]`
//! Applies background blur (and an optional text overlay) to one photo.

use linkcam_vision::effects::{blur_background, draw_text};
use linkcam_vision::{ImageView, PersonSegmenter};
use std::path::Path;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (models, input, output) = (&args[0], &args[1], &args[2]);
    let label = args.get(3);

    let mut decoder =
        jpeg_decoder::Decoder::new(std::io::BufReader::new(std::fs::File::open(input).unwrap()));
    let pixels = decoder.decode().unwrap();
    let info = decoder.info().unwrap();
    let (w, h) = (info.width as usize, info.height as usize);
    let mut rgba: Vec<u8> = pixels.as_chunks::<3>().0.iter().flat_map(|p| [p[0], p[1], p[2], 255]).collect();

    let segmenter = PersonSegmenter::load(Path::new(models)).expect("load segmentation model");
    let started = std::time::Instant::now();
    let mask = segmenter.mask(&ImageView { width: w, height: h, rgba: &rgba }).expect("segment");
    let segmented = started.elapsed();
    let person_share = mask.person.iter().filter(|p| **p > 0.5).count() as f32 / mask.person.len() as f32;
    blur_background(&mut rgba, w, h, &mask, 3);
    if let Some(text) = label {
        draw_text(&mut rgba, w, h, text, 3, 8);
    }
    println!(
        "{w}x{h}: segmentation {segmented:?}, blur+text {:?}, person area {:.0}%",
        started.elapsed() - segmented,
        person_share * 100.0
    );

    let file = std::io::BufWriter::new(std::fs::File::create(output).unwrap());
    let mut encoder = png::Encoder::new(file, w as u32, h as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header().unwrap().write_image_data(&rgba).unwrap();
}
