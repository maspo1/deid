//! 자체 OCR 원문 확인: `cargo run --release --example probe_lite -- <그림>`
fn main() {
    let img = image::open(std::env::args().nth(1).unwrap()).unwrap().to_rgba8();
    let t = std::time::Instant::now();
    let mut lines = deid::ocr::lite::recognize(&img);
    let ms = t.elapsed().as_secs_f64() * 1e3;
    lines.sort_by(|a, b| (a.words[0].y, a.words[0].x).partial_cmp(&(b.words[0].y, b.words[0].x)).unwrap());
    for l in &lines {
        let t: Vec<&str> = l.words.iter().map(|w| w.text.as_str()).collect();
        println!("({:>5.0},{:>5.0} h{:>3.0}) {}", l.words[0].x, l.words[0].y, l.words[0].h, t.join(" "));
    }
    println!("{} 줄, {ms:.0} ms", lines.len());
}
