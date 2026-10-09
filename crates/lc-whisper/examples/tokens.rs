use whisper_rs::*;
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let (pcm, _) = lc_core::audio::wav::read_wav(std::path::Path::new(&a[2])).unwrap();
    let ctx = WhisperContext::new_with_params(&a[1], WhisperContextParameters::default()).unwrap();
    let mut st = ctx.create_state().unwrap();
    let mut p = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    p.set_language(Some("pl"));
    p.set_token_timestamps(true);
    st.full(p, &pcm).unwrap();
    for seg in st.as_iter() {
        println!("SEG {:?}", seg.to_str_lossy());
        for i in 0..seg.n_tokens() {
            let t = seg.get_token(i).unwrap();
            let d = t.token_data();
            println!("  {:>6} {:?} {:?} {}-{}", d.id, t.to_bytes().map(|b| b.to_vec()).unwrap_or_default().iter().map(|&c| c as char).collect::<String>(), t.to_str_lossy().unwrap_or_default(), d.t0, d.t1);
        }
    }
}
