use super::*;
use futures_util::StreamExt;
use futures_util::stream;
use serde_json::json;
use std::sync::Mutex;

const PASSWORD: &str = "unit test password";

/// Tables served from memory.
struct Fixed(BTreeMap<String, Vec<Bytes>>);

impl TableSource for Fixed {
    fn copy_out(&mut self, t: &TableInfo) -> BoxStream<'_, Result<Bytes, sqlx::Error>> {
        let chunks = self.0.get(&t.name).cloned().unwrap_or_default();
        stream::iter(chunks.into_iter().map(Ok)).boxed()
    }
}

/// A restore target that keeps what it is given.
#[derive(Default)]
struct Recorder {
    settings: BTreeMap<String, Value>,
    docs: Vec<(String, Bytes)>,
    tables: BTreeMap<String, Vec<u8>>,
    current: Option<String>,
    z2m: Option<Bytes>,
}

impl Target for Recorder {
    async fn settings(&mut self, rows: &BTreeMap<String, Value>) -> Result<(), BackupError> {
        self.settings = rows.clone();
        Ok(())
    }

    async fn retained(&mut self, docs: Vec<(String, Bytes)>) -> Result<(), BackupError> {
        self.docs = docs;
        Ok(())
    }

    async fn begin_table(&mut self, name: &str) -> Result<(), BackupError> {
        self.current = Some(name.to_owned());
        self.tables.insert(name.to_owned(), Vec::new());
        Ok(())
    }

    async fn send(&mut self, data: Bytes) -> Result<(), BackupError> {
        let t = self.current.as_ref().unwrap();
        self.tables.get_mut(t).unwrap().extend_from_slice(&data);
        Ok(())
    }

    async fn end_table(&mut self) -> Result<u64, BackupError> {
        let t = self.current.take().unwrap();
        Ok(lines(&self.tables[&t]))
    }
}

fn lines(data: &[u8]) -> u64 {
    data.iter().filter(|b| **b == b'\n').count() as u64
}

fn random(n: usize) -> Vec<u8> {
    let mut v = vec![0u8; n];
    getrandom::getrandom(&mut v).unwrap();
    v
}

/// `COPY`-like text that zstd can't shrink: `rows` lines of random hex.
fn random_rows(rows: usize, width: usize) -> Vec<u8> {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = Vec::with_capacity(rows * (width * 2 + 1));
    for _ in 0..rows {
        for b in random(width) {
            out.push(HEX[usize::from(b >> 4)]);
            out.push(HEX[usize::from(b & 15)]);
        }
        out.push(b'\n');
    }
    out
}

struct Sample {
    manifest: Manifest,
    hub: HubFields,
    settings: BTreeMap<String, Value>,
    docs: Vec<(String, Bytes)>,
    tables: Vec<TableInfo>,
    data: BTreeMap<String, Vec<Bytes>>,
    z2m: Option<Bytes>,
}

impl Sample {
    /// Carries `zip` as Zigbee2MQTT's backup, the manifest saying so.
    fn with_z2m(mut self, zip: Vec<u8>) -> Sample {
        self.manifest.z2m = Some(Z2mManifest {
            bytes: zip.len() as u64,
        });
        self.z2m = Some(Bytes::from(zip));
        self
    }
}

/// Three tables: `big` spans two entries, `empty` has no rows, `small` two rows sent in
/// pieces that split a row.
fn sample(big_bytes: usize) -> Sample {
    let width = 500;
    let big = random_rows(big_bytes / (width * 2 + 1) + 1, width);
    let small = b"1\ttab\\there\n2\t\\N\n".to_vec();
    let data: BTreeMap<String, Vec<Bytes>> = [
        (
            "big".to_owned(),
            big.chunks(100_000).map(Bytes::copy_from_slice).collect(),
        ),
        ("empty".to_owned(), vec![]),
        (
            "small".to_owned(),
            vec![
                Bytes::copy_from_slice(&small[..5]),
                Bytes::copy_from_slice(&small[5..]),
            ],
        ),
    ]
    .into_iter()
    .collect();
    let tables: Vec<TableInfo> = data
        .iter()
        .map(|(name, chunks)| TableInfo {
            name: name.clone(),
            columns: vec!["a".into(), "b".into()],
            rows: chunks.iter().map(|c| lines(c)).sum(),
        })
        .collect();
    let docs = vec![
        (
            "nidoo/automation/f1".to_owned(),
            Bytes::from("{\"id\":\"f1\"}"),
        ),
        (
            "nidoo/ui/dashboards/d1".to_owned(),
            Bytes::from(vec![0u8, 255, 10]),
        ),
    ];
    let manifest = Manifest {
        format: FORMAT,
        nidoo_version: "1.2.3".into(),
        migration: 7,
        created_at: "2026-11-11T10:00:00Z".into(),
        host: "hub".into(),
        tables: tables
            .iter()
            .map(|t| {
                let m = TableManifest {
                    columns: t.columns.clone(),
                    rows: t.rows,
                };
                (t.name.clone(), m)
            })
            .collect(),
        retained: 2,
        flows: 1,
        dashboards: 1,
        history_bytes: 123,
        z2m: None,
    };
    Sample {
        manifest,
        hub: HubFields {
            z2m_base: Some("zigbee2mqtt".into()),
            timezone: Some("Europe/Paris".into()),
            ..HubFields::default()
        },
        settings: [("home".to_owned(), json!({"name": "Maison"}))]
            .into_iter()
            .collect(),
        docs,
        tables,
        data,
        z2m: None,
    }
}

async fn write_sample(s: &Sample, progress: &(dyn Fn(BackupPart, u64) + Sync)) -> Vec<u8> {
    let mut out = Vec::new();
    let contents = Contents {
        manifest: &s.manifest,
        hub: &s.hub,
        settings: &s.settings,
        z2m: s.z2m.as_ref(),
        docs: &s.docs,
        tables: &s.tables,
    };
    write_archive(
        crypt::seal(&mut out, PASSWORD),
        contents,
        Fixed(s.data.clone()),
        progress,
    )
    .await
    .unwrap();
    out
}

async fn restore(
    file: &[u8],
) -> Result<(Manifest, HubFields, BTreeMap<String, Value>, Recorder), BackupError> {
    let (tar, manifest) = open_archive(file, PASSWORD).await?;
    let mut rec = Recorder::default();
    let (hub, settings, z2m) = restore_into(tar, &manifest, &mut rec, &|_| {}).await?;
    rec.z2m = z2m;
    Ok((manifest, hub, settings, rec))
}

/// The plaintext tar, decrypted and decompressed by the standard libraries.
fn plaintext_entries(file: &[u8]) -> Vec<(String, Vec<u8>)> {
    let decryptor = age::Decryptor::new(file).unwrap();
    let identity =
        age::scrypt::Identity::new(age::secrecy::SecretString::from(PASSWORD.to_owned()));
    let plain = decryptor
        .decrypt(std::iter::once(&identity as &dyn age::Identity))
        .unwrap();
    let mut archive = ::tar::Archive::new(zstd::stream::read::Decoder::new(plain).unwrap());
    archive
        .entries()
        .unwrap()
        .map(|e| {
            let mut e = e.unwrap();
            let path = e.path().unwrap().to_string_lossy().into_owned();
            let mut data = Vec::new();
            std::io::Read::read_to_end(&mut e, &mut data).unwrap();
            (path, data)
        })
        .collect()
}

#[test]
fn manifest_json_has_the_documented_fields() {
    let m = sample(10).manifest;
    let v = serde_json::to_value(&m).unwrap();
    let keys: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        [
            "created_at",
            "dashboards",
            "flows",
            "format",
            "history_bytes",
            "host",
            "migration",
            "nidoo_version",
            "retained",
            "tables"
        ]
    );
    assert_eq!(
        v["tables"]["small"],
        json!({"columns": ["a", "b"], "rows": 2})
    );
    assert_eq!(serde_json::from_value::<Manifest>(v).unwrap(), m);
}

#[test]
fn copy_paths_round_trip() {
    assert_eq!(copy_path("entity_state", 1), "db/entity_state/000001.copy");
    assert_eq!(
        parse_copy_path("db/entity_state/000012.copy"),
        Some(("entity_state", 12))
    );
    for bad in [
        "db/t/1.copy",
        "db//000001.copy",
        "db/t/000001.txt",
        "db/a/b/000001.copy",
        "x/t/000001.copy",
    ] {
        assert_eq!(parse_copy_path(bad), None, "{bad}");
    }
}

#[test]
fn newer_backups_are_too_new() {
    let m = sample(10).manifest;
    assert_eq!(check_compatible(&m, 7), Ok(()));
    assert_eq!(check_compatible(&m, 8), Ok(()));
    assert_eq!(check_compatible(&m, 6), Err(BackupError::TooNew));
    let newer = Manifest {
        format: FORMAT + 1,
        ..m
    };
    assert_eq!(check_compatible(&newer, 7), Err(BackupError::TooNew));
    // A newer manifest this hub can't even parse.
    assert_eq!(
        parse_manifest(br#"{"format": 3, "parts": []}"#),
        Err(BackupError::TooNew)
    );
    assert_eq!(
        parse_manifest(br#"{"format": 1}"#),
        Err(BackupError::NotABackup)
    );
    assert_eq!(parse_manifest(b"[]"), Err(BackupError::NotABackup));
}

#[tokio::test]
async fn entries_are_laid_out_in_order() {
    let zip = random(9 * 1024 * 1024);
    let s = sample(MAX_ENTRY + 1000).with_z2m(zip.clone());
    let file = write_sample(&s, &|_, _| {}).await;
    let entries = plaintext_entries(&file);
    let names: Vec<&str> = entries.iter().map(|(p, _)| p.as_str()).collect();
    assert_eq!(
        names,
        [
            "manifest.json",
            "hub.json",
            "settings.json",
            "z2m/000001.zip",
            "z2m/000002.zip",
            "retained/000001.json",
            "db/big/000001.copy",
            "db/big/000002.copy",
            "db/small/000001.copy",
            "end.json"
        ]
    );
    assert_eq!(entries[3].1.len(), MAX_ENTRY);
    assert_eq!([&entries[3].1[..], &entries[4].1[..]].concat(), zip);
    assert_eq!(
        s.manifest.z2m,
        Some(Z2mManifest {
            bytes: 9 * 1024 * 1024
        })
    );
    let big: Vec<u8> = s.data["big"].concat();
    assert_eq!([&entries[6].1[..], &entries[7].1[..]].concat(), big);

    let json = |i: usize| serde_json::from_slice::<Value>(&entries[i].1).unwrap();
    assert_eq!(
        serde_json::from_value::<Manifest>(json(0)).unwrap(),
        s.manifest
    );
    assert_eq!(json(1)["z2m_base"], "zigbee2mqtt");
    assert_eq!(json(2), json!({"home": {"name": "Maison"}}));
    assert_eq!(
        json(5),
        json!([
            {"topic": "nidoo/automation/f1", "payload_b64": "eyJpZCI6ImYxIn0="},
            {"topic": "nidoo/ui/dashboards/d1", "payload_b64": "AP8K"}
        ])
    );
    assert_eq!(
        json(9),
        json!({"tables": {"big": s.manifest.tables["big"].rows, "empty": 0, "small": 2}})
    );
}

#[tokio::test]
async fn progress_counts_plaintext_bytes() {
    let s = sample(1000);
    let seen = Mutex::new(Vec::new());
    let file = write_sample(&s, &|part, bytes| seen.lock().unwrap().push((part, bytes))).await;
    let seen = seen.into_inner().unwrap();
    let parts: Vec<BackupPart> = seen.iter().map(|(p, _)| *p).collect();
    use BackupPart::*;
    assert_eq!(
        parts,
        [
            Settings, Settings, Settings, Retained, History, History, History
        ]
    );
    assert!(seen.windows(2).all(|w| w[0].1 < w[1].1), "{seen:?}");
    // The last report is every entry; the closing blocks follow.
    let tar_len: u64 = plaintext_entries(&file)
        .iter()
        .map(|(_, d)| (BLOCK + d.len().div_ceil(BLOCK) * BLOCK) as u64)
        .sum();
    assert_eq!(seen.last().unwrap().1, tar_len);
}

#[tokio::test]
async fn a_table_whose_rows_changed_is_refused() {
    let mut s = sample(1000);
    s.tables
        .iter_mut()
        .find(|t| t.name == "small")
        .unwrap()
        .rows = 3;
    let mut out = Vec::new();
    let contents = Contents {
        manifest: &s.manifest,
        hub: &s.hub,
        settings: &s.settings,
        z2m: s.z2m.as_ref(),
        docs: &s.docs,
        tables: &s.tables,
    };
    let err = write_archive(
        crypt::seal(&mut out, PASSWORD),
        contents,
        Fixed(s.data.clone()),
        &|_, _| {},
    )
    .await
    .unwrap_err();
    assert_eq!(
        err,
        BackupError::Database("table small: copied 2 rows, counted 3".into())
    );
}

#[tokio::test]
async fn restore_reads_back_what_was_written() {
    let s = sample(200_000);
    let file = write_sample(&s, &|_, _| {}).await;
    let (manifest, hub, settings, rec) = restore(&file).await.unwrap();
    assert_eq!(manifest, s.manifest);
    assert_eq!(hub, s.hub);
    assert_eq!(settings, s.settings);
    assert_eq!(rec.settings, s.settings);
    assert_eq!(rec.docs, s.docs);
    let expected: BTreeMap<String, Vec<u8>> = s
        .data
        .iter()
        .filter(|(_, chunks)| !chunks.is_empty())
        .map(|(name, chunks)| (name.clone(), chunks.concat()))
        .collect();
    assert_eq!(rec.tables, expected);
    assert_eq!(rec.z2m, None);
    assert!(
        !plaintext_entries(&file)
            .iter()
            .any(|(p, _)| p.starts_with("z2m/"))
    );

    let zip = random(9 * 1024 * 1024 + 5);
    let s = sample(10).with_z2m(zip.clone());
    let file = write_sample(&s, &|_, _| {}).await;
    let (manifest, .., rec) = restore(&file).await.unwrap();
    assert_eq!(manifest, s.manifest);
    assert_eq!(rec.z2m, Some(Bytes::from(zip)));
}

/// A format 1 file has no `z2m` key in its manifest and no `z2m/` entries.
#[tokio::test]
async fn a_format_1_file_still_restores() {
    let mut s = sample(10);
    s.manifest.format = 1;
    let file = write_sample(&s, &|_, _| {}).await;
    let entries = plaintext_entries(&file);
    let manifest: Value = serde_json::from_slice(&entries[0].1).unwrap();
    assert_eq!(manifest["format"], 1);
    assert!(manifest.get("z2m").is_none(), "{manifest}");
    let (manifest, .., rec) = restore(&file).await.unwrap();
    assert_eq!(manifest.format, 1);
    assert_eq!(manifest.z2m, None);
    assert_eq!(rec.z2m, None);
}

#[tokio::test]
async fn z2m_entries_out_of_order_are_refused() {
    let mut s = sample(10);
    s.manifest.tables = BTreeMap::new();
    let head = vec![
        (MANIFEST, serde_json::to_vec(&s.manifest).unwrap()),
        (HUB, b"{}".to_vec()),
        (SETTINGS, b"{}".to_vec()),
    ];
    let mut skipped = head.clone();
    skipped.push(("z2m/000002.zip", b"PK".to_vec()));
    let err = restore(&file_with(&skipped).await).await.err().unwrap();
    assert_eq!(err, unexpected("z2m/000002.zip"));

    // A zip the manifest doesn't account for.
    let mut stray = head;
    stray.push(("z2m/000001.zip", b"PK".to_vec()));
    stray.push(("retained/000001.json", b"[]".to_vec()));
    stray.push((END, br#"{"tables": {}}"#.to_vec()));
    assert!(restore(&file_with(&stray).await).await.is_err());
}

#[test]
fn z2m_paths_round_trip() {
    assert_eq!(z2m_path(1), "z2m/000001.zip");
    assert_eq!(parse_z2m_path("z2m/000012.zip"), Some(12));
    for bad in ["z2m/1.zip", "z2m/000001.json", "x/000001.zip", "z2m/a.zip"] {
        assert_eq!(parse_z2m_path(bad), None, "{bad}");
    }
}

/// Ruling A: 256 KiB holds the first age chunk (64 KiB + tag after the header) and more.
#[tokio::test]
async fn manifest_reads_from_the_head() {
    let s = sample(1024 * 1024);
    let file = write_sample(&s, &|_, _| {}).await;
    assert!(file.len() > 2 * 256 * 1024, "{}", file.len());
    let head = &file[..256 * 1024];
    assert_eq!(read_manifest(head, PASSWORD).await.unwrap(), s.manifest);
    assert_eq!(
        read_manifest(head, "wrong password").await,
        Err(BackupError::WrongPassword)
    );
    assert_eq!(
        read_manifest(&random(4096)[..], PASSWORD).await,
        Err(BackupError::NotABackup)
    );
}

/// Where each age payload chunk (64 KiB + 16-byte tag) starts; the header ends with
/// `--- <mac>\n` and a 16-byte nonce follows it.
fn chunk_starts(file: &[u8]) -> Vec<usize> {
    let mac = file.windows(5).position(|w| w == b"\n--- ").unwrap() + 5;
    let header_end = mac + file[mac..].iter().position(|b| *b == b'\n').unwrap() + 1;
    (header_end + 16..file.len())
        .step_by(64 * 1024 + 16)
        .collect()
}

/// No cut file restores: not at 70 %, not one byte short, not without its last chunk.
#[tokio::test]
async fn cut_files_never_restore() {
    let s = sample(300_000);
    let file = write_sample(&s, &|_, _| {}).await;
    let starts = chunk_starts(&file);
    assert!(starts.len() > 2, "{starts:?}");
    for cut in [file.len() * 7 / 10, file.len() - 1, *starts.last().unwrap()] {
        let result = restore(&file[..cut]).await;
        assert!(result.is_err(), "cut at {cut} of {} restored", file.len());
    }
}

/// The tar's end can be decoded before the stream's own end: zstd flushes it in a block of
/// its own, and the frame end that follows may fall in age's next chunk. A cut there only
/// shows when the stream is read to its end, past the tar's closing block.
#[tokio::test]
async fn an_error_after_the_tar_end_fails_the_restore() {
    struct Fails;
    impl AsyncRead for Fails {
        fn poll_read(
            self: Pin<&mut Self>,
            _: &mut Context<'_>,
            _: &mut ReadBuf<'_>,
        ) -> Poll<io::Result<()>> {
            Poll::Ready(Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "age file is truncated",
            )))
        }
    }
    let s = sample(1000);
    let file = write_sample(&s, &|_, _| {}).await;
    let mut tar = Vec::new();
    {
        let mut tw = TarWriter::new(&mut tar);
        for (path, data) in plaintext_entries(&file) {
            tw.entry(&path, &data).await.unwrap();
        }
        tw.finish().await.unwrap();
    }

    async fn restore_from<R: AsyncRead + Unpin>(
        input: R,
        manifest: &Manifest,
    ) -> Result<(HubFields, BTreeMap<String, Value>, Option<Bytes>), BackupError> {
        let mut reader = TarReader::new(input);
        reader.next().await.unwrap().unwrap();
        restore_into(reader, manifest, &mut Recorder::default(), &|_| {}).await
    }
    use tokio::io::AsyncReadExt;
    let whole = restore_from(tar.as_slice().chain(&[][..]), &s.manifest).await;
    assert!(whole.is_ok(), "{whole:?}");
    let err = restore_from(tar.as_slice().chain(Fails), &s.manifest)
        .await
        .unwrap_err();
    assert_eq!(err, ends_early());
}

/// A sealed file holding exactly `entries`.
async fn file_with(entries: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut tw = TarWriter::new(crypt::seal(&mut out, PASSWORD));
    for (path, data) in entries {
        tw.entry(path, data).await.unwrap();
    }
    tw.finish().await.unwrap().shutdown().await.unwrap();
    out
}

#[test]
fn retained_entries_stay_under_the_limit() {
    assert_eq!(retained_entries(&[], 100).unwrap(), [b"[]".to_vec()]);

    let docs: Vec<(String, Bytes)> = (0..20)
        .map(|i| {
            (
                format!("nidoo/automation/f{i}"),
                Bytes::from(vec![i as u8; 40]),
            )
        })
        .collect();
    let limit = 300;
    let entries = retained_entries(&docs, limit).unwrap();
    assert!(entries.len() > 3, "{}", entries.len());
    let mut back = Vec::new();
    for e in &entries {
        assert!(e.len() <= limit, "{} > {limit}", e.len());
        let part: Vec<RetainedDoc> = serde_json::from_slice(e).unwrap();
        assert!(!part.is_empty());
        back.extend(part.into_iter().map(|d| {
            (
                d.topic,
                Bytes::from(STANDARD.decode(d.payload_b64).unwrap()),
            )
        }));
    }
    assert_eq!(back, docs);
    // Compact JSON, no pretty-printing.
    assert!(!entries[0].contains(&b'\n'));

    let huge = [(
        "nidoo/ui/dashboards/big".to_owned(),
        Bytes::from(vec![0u8; 400]),
    )];
    let err = retained_entries(&huge, limit).unwrap_err();
    assert_eq!(
        err.to_string(),
        "broker: retained topic nidoo/ui/dashboards/big is too large to back up (400 bytes)"
    );
}

#[tokio::test]
async fn retained_entries_are_read_in_order() {
    let mut s = sample(10);
    s.manifest.tables = BTreeMap::new();
    let head = vec![
        (MANIFEST, serde_json::to_vec(&s.manifest).unwrap()),
        (HUB, b"{}".to_vec()),
        (SETTINGS, b"{}".to_vec()),
    ];
    let doc = |topic: &str| json!([{"topic": topic, "payload_b64": "e30="}]).to_string();
    let end = (END, br#"{"tables": {}}"#.to_vec());

    let mut two = head.clone();
    two.push(("retained/000001.json", doc("a/1").into_bytes()));
    two.push(("retained/000002.json", doc("a/2").into_bytes()));
    two.push(end.clone());
    let (.., rec) = restore(&file_with(&two).await).await.unwrap();
    let topics: Vec<&str> = rec.docs.iter().map(|(t, _)| t.as_str()).collect();
    assert_eq!(topics, ["a/1", "a/2"]);
    assert_eq!(rec.docs[0].1, Bytes::from("{}"));

    let mut gap = head.clone();
    gap.push(("retained/000001.json", doc("a/1").into_bytes()));
    gap.push(("retained/000003.json", doc("a/3").into_bytes()));
    gap.push(end.clone());
    let err = restore(&file_with(&gap).await).await.err().unwrap();
    assert_eq!(err, unexpected("retained/000003.json"));

    let mut none = head;
    none.push(end);
    let err = restore(&file_with(&none).await).await.err().unwrap();
    assert_eq!(err, unexpected(END));
}

#[test]
fn restored_debug_shows_only_setting_keys() {
    let s = sample(10);
    let restored = Restored {
        hub: s.hub,
        settings: [("backup".to_owned(), json!({"password": "hunter2-secret"}))]
            .into_iter()
            .collect(),
        manifest: s.manifest,
        z2m: None,
    };
    let text = format!("{restored:?}");
    assert!(text.contains("\"backup\""), "{text}");
    assert!(!text.contains("hunter2-secret"), "{text}");
}

#[test]
fn restored_debug_hides_the_zip() {
    let s = sample(10);
    let restored = Restored {
        hub: s.hub,
        settings: BTreeMap::new(),
        manifest: s.manifest,
        z2m: Some(Bytes::from_static(b"PK\x03\x04 network key")),
    };
    let text = format!("{restored:?}");
    assert!(!text.contains("PK"), "{text}");
    assert!(!text.contains("network key"), "{text}");
    assert!(text.contains("z2m: Some(16)"), "{text}");
}

/// A tar ending without `end.json`, or with fewer rows than the manifest says, is damaged.
#[tokio::test]
async fn missing_rows_are_found() {
    let mut s = sample(10);
    s.manifest.tables = [(
        "t".to_owned(),
        TableManifest {
            columns: vec!["a".into()],
            rows: 12,
        },
    )]
    .into_iter()
    .collect();
    let head = vec![
        (MANIFEST, serde_json::to_vec(&s.manifest).unwrap()),
        (HUB, b"{}".to_vec()),
        (SETTINGS, b"{}".to_vec()),
        ("retained/000001.json", b"[]".to_vec()),
    ];
    let end = (END, br#"{"tables": {"t": 12}}"#.to_vec());

    let mut short = head.clone();
    short.push(("db/t/000001.copy", b"1\n".repeat(10)));
    short.push(end.clone());
    let err = restore(&file_with(&short).await).await.err().unwrap();
    assert_eq!(
        err.to_string(),
        "the backup is damaged: table t: 10 rows, expected 12"
    );

    let mut no_rows = head.clone();
    no_rows.push(end.clone());
    let err = restore(&file_with(&no_rows).await).await.err().unwrap();
    assert_eq!(err, rows_differ("t", 0, 12));

    let mut no_end = head.clone();
    no_end.push(("db/t/000001.copy", b"1\n".repeat(12)));
    let err = restore(&file_with(&no_end).await).await.err().unwrap();
    assert_eq!(err, ends_early());

    let mut gap = head;
    gap.push(("db/t/000001.copy", b"1\n".repeat(6)));
    gap.push(("db/t/000003.copy", b"1\n".repeat(6)));
    gap.push(end);
    assert!(restore(&file_with(&gap).await).await.is_err());
}

/// Spawned jobs need these futures to be `Send`; this only has to compile.
#[allow(dead_code)]
fn futures_are_send(pool: &PgPool, broker: &dyn RetainedBroker, layout: Layout) {
    fn send<T: Send>(_: T) {}
    let sources = Sources {
        pool,
        broker,
        layout,
        hub: HubFields::default(),
        z2m: None,
    };
    send(write_backup(sources, Vec::new(), "pw", &|_, _| {}));
    send(read_backup(&b""[..], "pw", pool, broker, &|_, _| {}));
    send(read_manifest(&b""[..], "pw"));
}
