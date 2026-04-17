//! End-to-end integration: build a tiny WAV, push it through the job
//! executor as `remux` and `@null` targets, and verify round-trip.
//!
//! The WAV is synthesized here (no test fixtures) so the test is
//! self-contained and platform-agnostic.

use std::io::Write;

use oxideav_codec::CodecRegistry;
use oxideav_container::ContainerRegistry;
use oxideav_job::{Executor, Job};
use oxideav_source::SourceRegistry;

fn build_pcm_wav(path: &std::path::Path, sample_rate: u32, ms: u32) {
    // Minimal RIFF/WAV with 16-bit mono PCM.
    let n_samples = (sample_rate as u64 * ms as u64 / 1000) as u32;
    let byte_rate = sample_rate * 2;
    let data_sz = n_samples * 2;
    let riff_sz = 36 + data_sz;
    let mut f = std::fs::File::create(path).unwrap();
    f.write_all(b"RIFF").unwrap();
    f.write_all(&riff_sz.to_le_bytes()).unwrap();
    f.write_all(b"WAVE").unwrap();
    f.write_all(b"fmt ").unwrap();
    f.write_all(&16u32.to_le_bytes()).unwrap();
    f.write_all(&1u16.to_le_bytes()).unwrap(); // PCM
    f.write_all(&1u16.to_le_bytes()).unwrap(); // mono
    f.write_all(&sample_rate.to_le_bytes()).unwrap();
    f.write_all(&byte_rate.to_le_bytes()).unwrap();
    f.write_all(&2u16.to_le_bytes()).unwrap(); // block align
    f.write_all(&16u16.to_le_bytes()).unwrap(); // bits per sample
    f.write_all(b"data").unwrap();
    f.write_all(&data_sz.to_le_bytes()).unwrap();
    // Silence.
    for _ in 0..n_samples {
        f.write_all(&0i16.to_le_bytes()).unwrap();
    }
}

fn registries() -> (CodecRegistry, ContainerRegistry) {
    let mut codecs = CodecRegistry::new();
    let mut containers = ContainerRegistry::new();
    oxideav_basic::register_codecs(&mut codecs);
    oxideav_basic::register_containers(&mut containers);
    (codecs, containers)
}

fn tmp_dir(name: &str) -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("oxideav_job_{name}"));
    let _ = std::fs::create_dir_all(&p);
    p
}

#[test]
fn remux_wav_to_wav_copy_path() {
    let dir = tmp_dir("remux_copy");
    let src = dir.join("in.wav");
    let dst = dir.join("out.wav");
    let _ = std::fs::remove_file(&dst);
    build_pcm_wav(&src, 8_000, 100);

    let (codecs, containers) = registries();
    let sources = SourceRegistry::with_defaults();
    let json = format!(
        r#"{{ "{}": {{ "audio": [{{"from": "{}"}}] }} }}"#,
        dst.display().to_string().replace('\\', "\\\\"),
        src.display().to_string().replace('\\', "\\\\"),
    );
    let job = Job::from_json(&json).expect("parse job");
    let stats = Executor::new(&job, &codecs, &containers, &sources)
        .run()
        .expect("run job");
    // Output file exists and matches input size within a small fudge.
    let in_sz = std::fs::metadata(&src).unwrap().len();
    let out_sz = std::fs::metadata(&dst).unwrap().len();
    assert!(
        (out_sz as i64 - in_sz as i64).abs() < 256,
        "size mismatch: in={in_sz} out={out_sz}"
    );
    assert!(
        stats.packets_copied > 0 || stats.packets_read > 0,
        "no packets flowed: {stats:?}"
    );
}

#[test]
fn null_sink_accepts_anything() {
    let dir = tmp_dir("remux_null");
    let src = dir.join("in.wav");
    build_pcm_wav(&src, 8_000, 50);

    let (codecs, containers) = registries();
    let sources = SourceRegistry::with_defaults();
    let json = format!(
        r#"{{ "@null": {{ "audio": [{{"from": "{}"}}] }} }}"#,
        src.display().to_string().replace('\\', "\\\\"),
    );
    let job = Job::from_json(&json).expect("parse");
    let stats = Executor::new(&job, &codecs, &containers, &sources)
        .run()
        .expect("run");
    assert!(stats.packets_read > 0, "no packets read: {stats:?}");
}

#[test]
fn rejects_cycle_before_run() {
    let job = Job::from_json(
        r#"{
            "@a": {"all": [{"from": "@b"}]},
            "@b": {"all": [{"from": "@a"}]},
            "out.wav": {"audio": [{"from": "@a"}]}
        }"#,
    )
    .unwrap();
    let (codecs, containers) = registries();
    let sources = SourceRegistry::with_defaults();
    let err = Executor::new(&job, &codecs, &containers, &sources)
        .run()
        .expect_err("should detect cycle");
    assert!(
        format!("{err}").contains("cycle"),
        "unexpected error: {err}"
    );
}

#[test]
fn audio_filter_chain_runs() {
    let dir = tmp_dir("filter_chain");
    let src = dir.join("in.wav");
    let dst = dir.join("out.wav");
    let _ = std::fs::remove_file(&dst);
    build_pcm_wav(&src, 8_000, 50);

    let (codecs, containers) = registries();
    let sources = SourceRegistry::with_defaults();
    // volume filter + re-encode to pcm_s16le. The input is silence so we
    // can't check for content change, but the pipeline must run cleanly.
    let json = format!(
        r#"{{
            "{}": {{
                "audio": [{{
                    "filter": "volume",
                    "params": {{"gain_db": -3}},
                    "input": {{"from": "{}"}},
                    "codec": "pcm_s16le"
                }}]
            }}
        }}"#,
        dst.display().to_string().replace('\\', "\\\\"),
        src.display().to_string().replace('\\', "\\\\"),
    );
    let job = Job::from_json(&json).expect("parse");
    let stats = Executor::new(&job, &codecs, &containers, &sources)
        .run()
        .expect("run");
    assert!(stats.frames_decoded > 0);
    assert!(stats.packets_encoded > 0);
    assert!(std::fs::metadata(&dst).unwrap().len() > 100);
}

#[test]
fn dry_run_describe_contains_output() {
    let job = Job::from_json(
        r#"{
            "out.wav": {
                "audio": [{"from": "in.wav"}]
            }
        }"#,
    )
    .unwrap();
    let dag = job.to_dag().unwrap();
    let desc = dag.describe();
    assert!(desc.contains("output: out.wav"), "{desc}");
    assert!(desc.contains("demuxer(in.wav)"), "{desc}");
}
