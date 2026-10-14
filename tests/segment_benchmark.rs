//! Explicit benchmark with self-generated speech only. Never uses user recordings or Pi.
use omm::{
    adapters::asr::{codec::AudioCodec, LocalAsrAdapter},
    core::{contracts::*, paths::private_dir, store::hash_file},
};
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use uuid::Uuid;
fn samples(root: &Path, voice: &str, text: &str) -> Vec<i16> {
    let input = root.join(format!("{voice}.txt"));
    std::fs::write(&input, text).unwrap();
    let output = root.join(format!("{voice}.wav"));
    assert!(Command::new("/usr/bin/say")
        .args(["-v", voice, "-r", "170", "-f"])
        .arg(input)
        .arg("-o")
        .arg(&output)
        .args(["--file-format=WAVE", "--data-format=LEI16@48000"])
        .status()
        .unwrap()
        .success());
    let mut r = hound::WavReader::open(output).unwrap();
    assert_eq!(r.spec().channels, 1);
    assert_eq!(r.spec().sample_rate, 48000);
    let samples: Vec<_> = r.samples::<i16>().map(|s| s.unwrap()).collect();
    assert!(
        !samples.is_empty(),
        "installed speech voice produced no samples"
    );
    samples
}
fn fixture(root: &Path, case: (&str, u64, bool, bool, bool), a: &[i16], b: &[i16]) -> AudioSegment {
    let (name, seconds, dual, quiet, noise) = case;
    let path = root.join(format!("{name}.wav"));
    let mut w = hound::WavWriter::create(
        &path,
        hound::WavSpec {
            channels: 2,
            sample_rate: 48000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )
    .unwrap();
    let mut random = 1u32;
    for i in 0..(seconds * 48000) as usize {
        random = random.wrapping_mul(1664525).wrapping_add(1013904223);
        let white = ((random >> 16) as i16) / 16;
        let mic = if noise && dual {
            (a[i % a.len()] as i32 + white as i32 / 2).clamp(-32768, 32767) as i16
        } else if noise {
            white
        } else if quiet {
            0
        } else if dual || name == "continuous20" || i % (20 * 48000) < 10 * 48000 {
            a[i % a.len()]
        } else {
            b[i % b.len()]
        };
        let system = if dual && !quiet {
            (b[i % b.len()] as i32 + if noise { white as i32 / 2 } else { 0 }).clamp(-32768, 32767)
                as i16
        } else {
            0
        };
        w.write_sample(mic).unwrap();
        w.write_sample(system).unwrap();
    }
    w.finalize().unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    AudioSegment {
        capture: None,
        sha256: hash_file(&path).unwrap(),
        path,
        duration_ms: seconds * 1000,
    }
}
fn release_worker(path: Option<PathBuf>) -> std::result::Result<PathBuf, &'static str> {
    let path =
        path.ok_or("set explicit OMM_REAL_ASR_WORKER from cargo build --locked --release")?;
    let path = path
        .canonicalize()
        .map_err(|_| "release worker does not exist")?;
    if path
        .parent()
        .and_then(Path::file_name)
        .and_then(|s| s.to_str())
        != Some("release")
    {
        return Err("benchmark worker must be the explicitly built target/release binary");
    }
    Ok(path)
}
#[derive(Debug, Clone, Copy)]
struct CpuSample {
    owned: f64,
    other_product: f64,
}
fn sampler(stop: Arc<AtomicBool>) -> std::thread::JoinHandle<Vec<CpuSample>> {
    let parent = std::process::id();
    std::thread::spawn(move || {
        let mut samples = vec![];
        while !stop.load(Ordering::Relaxed) {
            let output = Command::new("/bin/ps")
                .args(["-A", "-o", "pid=,ppid=,%cpu=,comm="])
                .output()
                .unwrap();
            let rows = String::from_utf8(output.stdout)
                .unwrap()
                .lines()
                .filter_map(|l| {
                    let mut p = l.split_whitespace();
                    Some((
                        p.next()?.parse::<u32>().ok()?,
                        p.next()?.parse::<u32>().ok()?,
                        p.next()?.parse::<f64>().ok()?,
                        p.collect::<Vec<_>>().join(" "),
                    ))
                })
                .collect::<Vec<_>>();
            let mut owned = std::collections::HashSet::from([parent]);
            loop {
                let before = owned.len();
                for (pid, ppid, _, _) in &rows {
                    if owned.contains(ppid) {
                        owned.insert(*pid);
                    }
                }
                if owned.len() == before {
                    break;
                }
            }
            let cores = std::thread::available_parallelism().unwrap().get() as f64;
            let other_product = rows
                .iter()
                .filter(|(pid, _, _, name)| {
                    !owned.contains(pid)
                        && Path::new(name)
                            .file_name()
                            .and_then(|s| s.to_str())
                            .is_some_and(|s| matches!(s, "omm" | "omm-capture" | "omm-asr-worker"))
                })
                .map(|(_, _, cpu, _)| cpu)
                .sum::<f64>()
                / cores;
            samples.push(CpuSample {
                owned: rows
                    .iter()
                    .filter(|(pid, _, _, _)| owned.contains(pid))
                    .map(|(_, _, cpu, _)| cpu)
                    .sum::<f64>()
                    / cores,
                other_product,
            });
            std::thread::sleep(Duration::from_millis(200));
        }
        samples
    })
}
#[tokio::test]
#[ignore = "explicit existing pinned model + locked codec + local synthesized speech benchmark"]
async fn segment_real_model_self_generated_20_60s_threads_bitrates() {
    let root =
        PathBuf::from(std::env::var_os("OMM_BENCH_ROOT").expect("set dedicated output directory"));
    private_dir(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let codec = AudioCodec::from_manifest(&PathBuf::from(
        std::env::var_os("OMM_CODEC_TEST_MANIFEST").unwrap(),
    ))
    .unwrap();
    let models = PathBuf::from(std::env::var_os("OMM_REAL_ASR_MODELS").unwrap());
    let library = PathBuf::from(std::env::var_os("OMM_REAL_ASR_LIBRARY").unwrap());
    let worker =
        release_worker(std::env::var_os("OMM_REAL_ASR_WORKER").map(PathBuf::from)).unwrap();
    let worker_hash = hash_file(&worker).unwrap();
    let adapter = LocalAsrAdapter::new(worker.clone()).unwrap();
    let chinese=samples(&root,"Tingting","今天会议讨论项目计划。我们需要在明天完成测试，确认音频和会议记录完整。张经理负责检查结果，李工程师负责准备发布。下次会议安排在星期五上午九点。");
    let english=samples(&root,"Samantha","Today the meeting covers our project plan. We need to finish the tests tomorrow and confirm that audio and meeting notes are complete. Alex will check the results and Sam will prepare the release. Our next meeting is Friday morning at nine.");
    let cases = [
        ("dual20", 20, true, false, false),
        ("mono20", 20, false, false, false),
        ("dual60", 60, true, false, false),
        ("mono60", 60, false, false, false),
        ("silence20", 20, false, true, false),
        ("noise20", 20, false, false, true),
        ("speech-noise20", 20, true, false, true),
        ("continuous20", 20, false, false, false),
    ];
    let selected_case = std::env::var("OMM_BENCH_CASE").ok();
    let mut outputs = vec![];
    for (name, seconds, dual, quiet, noise) in cases {
        if selected_case
            .as_deref()
            .is_some_and(|selected| selected != name)
        {
            continue;
        }
        let original = fixture(
            &root,
            (name, seconds, dual, quiet, noise),
            &chinese,
            &english,
        );
        let profiles = if name == "dual20" {
            vec![
                (1, 32000),
                (1, 48000),
                (1, 64000),
                (2, 32000),
                (2, 48000),
                (2, 64000),
                (3, 32000),
                (3, 48000),
                (3, 64000),
            ]
        } else {
            vec![(2, 48000), (3, 48000)]
        };
        for (threads, bitrate) in profiles {
            let profile = root.join(format!("{name}-{threads}-{bitrate}"));
            private_dir(&profile).unwrap();
            let work = profile.join("workspace");
            private_dir(&work).unwrap();
            let config = ConfigSnapshot {
                audio: Default::default(),
                asr: AsrConfig {
                    library_path: library.clone(),
                    encoder_path: models.join("fire-red-asr-int8/encoder.int8.onnx"),
                    decoder_path: models.join("fire-red-asr-int8/decoder.int8.onnx"),
                    tokens_path: models.join("fire-red-asr-int8/tokens.txt"),
                    vad_path: models.join("silero-vad/silero_vad.onnx"),
                    threads,
                    // Existing product timeout: unchanged; tiny benchmarks normally finish far earlier.
                    timeout_ms: 7_200_000,
                    model_version: "sherpa-onnx-1.13.2-fire-red-asr-large-zh_en-2025-02-16-int8"
                        .into(),
                },
                ..Default::default()
            };
            let context = TaskContext {
                session_id: Uuid::new_v4(),
                task_id: Uuid::new_v4(),
                attempt: 1,
                lease: Lease {
                    token: Uuid::new_v4(),
                    expires_at_ms: (std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_millis() as i64)
                        + 300000,
                },
                meeting_dir: root.clone(),
                work_dir: work,
                config,
            };
            let stop = Arc::new(AtomicBool::new(false));
            let cpu = sampler(stop.clone());
            let start = Instant::now();
            let receipt = codec
                .archive(&context, &original, 0, bitrate)
                .await
                .unwrap();
            let encode = start.elapsed().as_secs_f64();
            let decoded = codec.decode_sources(&receipt, &context).await.unwrap();
            let start = Instant::now();
            let result = adapter
                .transcribe(AsrRequest {
                    context: context.clone(),
                    segments: vec![decoded],
                })
                .await;
            let asr = start.elapsed().as_secs_f64();
            adapter.drain().await.unwrap();
            codec.drain().await.unwrap();
            stop.store(true, Ordering::Relaxed);
            let cpu = cpu.join().unwrap();
            let transcript = result.unwrap();
            assert_eq!(transcript.duration_ms, seconds * 1000);
            assert!(transcript
                .segments
                .iter()
                .all(|s| s.start_ms <= s.end_ms && s.end_ms <= seconds * 1000));
            let mic = transcript
                .segments
                .iter()
                .filter(|s| s.source == AudioSource::Mic)
                .count();
            let system = transcript
                .segments
                .iter()
                .filter(|s| s.source == AudioSource::System)
                .count();
            if !quiet && (!noise || dual) {
                assert!(mic > 0);
                if dual {
                    assert!(system > 0);
                }
            } else {
                assert_eq!(mic + system, 0);
            }
            assert_eq!(
                hash_file(&worker).unwrap(),
                worker_hash,
                "benchmark worker changed during run"
            );
            let row = serde_json::json!({"workerSha256":worker_hash,"workerPath":worker,"buildProfile":"explicit-release","testRunnerDebugAssertions":cfg!(debug_assertions),"case":name,"seconds":seconds,"threads":threads,"bitrate":bitrate,"encodingSeconds":encode,"asrSeconds":asr,"realTimeFactor":asr/seconds as f64,"cpuMeanWholeMachine":cpu.iter().map(|c|c.owned).sum::<f64>()/cpu.len().max(1) as f64,"cpuPeakWholeMachine":cpu.iter().map(|c|c.owned).fold(0.0,f64::max),"cpuSamples":cpu.len(),"cpuOtherProductMeanWholeMachine":cpu.iter().map(|c|c.other_product).sum::<f64>()/cpu.len().max(1) as f64,"cpuAllProductPeakWholeMachine":cpu.iter().map(|c|c.owned+c.other_product).fold(0.0,f64::max),"micSpans":mic,"systemSpans":system,"textCharacters":transcript.segments.iter().map(|s|s.text.chars().count()).sum::<usize>(),"transcriptSha256":omm::core::store::hash_bytes(&serde_json::to_vec(&transcript).unwrap())});
            println!("BENCH {row}");
            std::fs::write(
                context.work_dir.join("transcript.json"),
                serde_json::to_vec_pretty(&transcript).unwrap(),
            )
            .unwrap();
            outputs.push(row);
            std::fs::write(
                root.join("results.json"),
                serde_json::to_vec_pretty(&outputs).unwrap(),
            )
            .unwrap();
        }
    }
    assert!(!outputs.is_empty(), "unknown benchmark case");
}

#[tokio::test]
#[ignore = "explicit locked old/new workers, generated WAV baseline and overlapping encoder CPU sample"]
async fn segment_real_model_wav_baseline_and_encoder_overlap() {
    let root = PathBuf::from(std::env::var_os("OMM_BENCH_ROOT").unwrap());
    private_dir(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let models = PathBuf::from(std::env::var_os("OMM_REAL_ASR_MODELS").unwrap());
    let library = PathBuf::from(std::env::var_os("OMM_REAL_ASR_LIBRARY").unwrap());
    let codec = AudioCodec::from_manifest(&PathBuf::from(
        std::env::var_os("OMM_CODEC_TEST_MANIFEST").unwrap(),
    ))
    .unwrap();
    let new_worker =
        release_worker(std::env::var_os("OMM_REAL_ASR_WORKER").map(PathBuf::from)).unwrap();
    let old_worker = PathBuf::from(std::env::var_os("OMM_BASELINE_ASR_WORKER").unwrap())
        .canonicalize()
        .unwrap();
    let chinese=samples(&root,"Tingting","今天会议讨论项目计划。我们需要在明天完成测试，确认音频和会议记录完整。张经理负责检查结果，李工程师负责准备发布。下次会议安排在星期五上午九点。");
    let english=samples(&root,"Samantha","Today the meeting covers our project plan. We need to finish the tests tomorrow and confirm that audio and meeting notes are complete. Alex will check the results and Sam will prepare the release. Our next meeting is Friday morning at nine.");
    let selected_case = std::env::var("OMM_BENCH_CASE").ok();
    let mut outputs = vec![];
    for (name, seconds, dual) in [
        ("dual20", 20, true),
        ("mono20", 20, false),
        ("dual60", 60, true),
        ("mono60", 60, false),
        ("dual60-overlap", 60, true),
    ] {
        if selected_case
            .as_deref()
            .is_some_and(|selected| selected != name)
        {
            continue;
        }
        let original = fixture(
            &root,
            (name, seconds, dual, false, false),
            &chinese,
            &english,
        );
        let profiles = if name.ends_with("overlap") {
            vec![("new", 3, &new_worker)]
        } else {
            vec![
                ("old", 2, &old_worker),
                ("new", 2, &new_worker),
                ("new", 3, &new_worker),
            ]
        };
        for (version, threads, worker) in profiles {
            let worker_hash = hash_file(worker).unwrap();
            let adapter = LocalAsrAdapter::new(worker.clone()).unwrap();
            let profile = root.join(format!("{name}-{version}-{threads}"));
            private_dir(&profile).unwrap();
            let work = profile.join("workspace");
            private_dir(&work).unwrap();
            let context = TaskContext {
                session_id: Uuid::new_v4(),
                task_id: Uuid::new_v4(),
                attempt: 1,
                lease: Lease {
                    token: Uuid::new_v4(),
                    expires_at_ms: chrono::Utc::now().timestamp_millis() + 600000,
                },
                meeting_dir: root.clone(),
                work_dir: work,
                config: ConfigSnapshot {
                    audio: Default::default(),
                    asr: AsrConfig {
                        library_path: library.clone(),
                        encoder_path: models.join("fire-red-asr-int8/encoder.int8.onnx"),
                        decoder_path: models.join("fire-red-asr-int8/decoder.int8.onnx"),
                        tokens_path: models.join("fire-red-asr-int8/tokens.txt"),
                        vad_path: models.join("silero-vad/silero_vad.onnx"),
                        threads,
                        timeout_ms: 7_200_000,
                        model_version:
                            "sherpa-onnx-1.13.2-fire-red-asr-large-zh_en-2025-02-16-int8".into(),
                    },
                    ..Default::default()
                },
            };
            let mut encoder_context = context.clone();
            encoder_context.task_id = Uuid::new_v4();
            encoder_context.work_dir = profile.join("encoder");
            private_dir(&encoder_context.work_dir).unwrap();
            let stop = Arc::new(AtomicBool::new(false));
            let cpu = sampler(stop.clone());
            let start = Instant::now();
            let operation = adapter.transcribe(AsrRequest {
                context: context.clone(),
                segments: vec![original.clone()],
            });
            let encoding = async {
                let start = Instant::now();
                if name.ends_with("overlap") {
                    for sequence in 0..12 {
                        codec
                            .archive(&encoder_context, &original, sequence, 48000)
                            .await
                            .unwrap();
                    }
                }
                start.elapsed().as_secs_f64()
            };
            let (result, encoding_seconds) = tokio::join!(operation, encoding);
            let seconds_elapsed = start.elapsed().as_secs_f64();
            adapter.drain().await.unwrap();
            codec.drain().await.unwrap();
            stop.store(true, Ordering::Relaxed);
            let cpu = cpu.join().unwrap();
            let transcript = result.unwrap();
            assert_eq!(transcript.duration_ms, seconds * 1000);
            assert!(transcript
                .segments
                .iter()
                .all(|s| s.start_ms < s.end_ms && s.end_ms <= seconds * 1000));
            assert!(transcript
                .segments
                .iter()
                .any(|s| s.source == AudioSource::Mic));
            if dual {
                assert!(transcript
                    .segments
                    .iter()
                    .any(|s| s.source == AudioSource::System));
            }
            assert_eq!(hash_file(worker).unwrap(), worker_hash);
            let row = serde_json::json!({"version":version,"workerSha256":worker_hash,"workerPath":worker,"buildProfile":"explicit-release","testRunnerDebugAssertions":cfg!(debug_assertions),"inputKind":"wav","case":name,"seconds":seconds,"threads":threads,"wallSeconds":seconds_elapsed,"encodingSeconds":encoding_seconds,"realTimeFactor":seconds_elapsed/seconds as f64,"cpuMeanWholeMachine":cpu.iter().map(|c|c.owned).sum::<f64>()/cpu.len().max(1) as f64,"cpuPeakWholeMachine":cpu.iter().map(|c|c.owned).fold(0.0,f64::max),"cpuSamples":cpu.len(),"cpuOtherProductMeanWholeMachine":cpu.iter().map(|c|c.other_product).sum::<f64>()/cpu.len().max(1) as f64,"cpuAllProductPeakWholeMachine":cpu.iter().map(|c|c.owned+c.other_product).fold(0.0,f64::max),"micSpans":transcript.segments.iter().filter(|s|s.source==AudioSource::Mic).count(),"systemSpans":transcript.segments.iter().filter(|s|s.source==AudioSource::System).count(),"textCharacters":transcript.segments.iter().map(|s|s.text.chars().count()).sum::<usize>(),"transcriptSha256":omm::core::store::hash_bytes(&serde_json::to_vec(&transcript).unwrap())});
            println!("RAW_BENCH {row}");
            outputs.push(row);
            std::fs::write(
                context.work_dir.join("transcript.json"),
                serde_json::to_vec_pretty(&transcript).unwrap(),
            )
            .unwrap();
            std::fs::write(
                root.join("results.json"),
                serde_json::to_vec_pretty(&outputs).unwrap(),
            )
            .unwrap();
        }
    }
    assert!(!outputs.is_empty(), "unknown benchmark case");
}

#[test]
fn benchmark_worker_requires_explicit_release_path_not_test_binary_profile() {
    assert!(release_worker(None).is_err());
    let tmp = tempfile::tempdir().unwrap();
    let debug = tmp.path().join("debug/omm-asr-worker");
    std::fs::create_dir_all(debug.parent().unwrap()).unwrap();
    std::fs::write(&debug, b"fixture").unwrap();
    assert!(release_worker(Some(debug)).is_err());
    let release = tmp.path().join("release/omm-asr-worker");
    std::fs::create_dir_all(release.parent().unwrap()).unwrap();
    std::fs::write(&release, b"fixture").unwrap();
    assert_eq!(
        release_worker(Some(release.clone())).unwrap(),
        release.canonicalize().unwrap()
    );
}
