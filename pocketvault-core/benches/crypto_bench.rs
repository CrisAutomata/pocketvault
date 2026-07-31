use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use pocketvault_core::{
    crypto::{decrypt, derive_key, encrypt, generate_vault_key, KdfParams},
    pv_format::{read_pv, write_pv, JobControl, PvMetadata},
};
use std::io::Cursor;

// ── AES-256-GCM ───────────────────────────────────────────────────────────────

fn bench_aes_gcm(c: &mut Criterion) {
    let key = generate_vault_key();
    let mut group = c.benchmark_group("aes_gcm");

    for size in [1_024usize, 64 * 1_024, 1_024 * 1_024, 10 * 1_024 * 1_024] {
        let data = vec![0xABu8; size];

        group.bench_with_input(BenchmarkId::new("encrypt", size), &size, |b, _| {
            b.iter(|| encrypt(&key, &data).unwrap());
        });

        let (ct, nonce) = encrypt(&key, &data).unwrap();
        group.bench_with_input(BenchmarkId::new("decrypt", size), &size, |b, _| {
            b.iter(|| decrypt(&key, &ct, &nonce).unwrap());
        });
    }
    group.finish();
}

// ── .pv file write / read (chunk-based) ──────────────────────────────────────

fn bench_pv_file(c: &mut Criterion) {
    let key = generate_vault_key();
    let mut group = c.benchmark_group("pv_file");

    for size in [64 * 1_024usize, 1_024 * 1_024, 10 * 1_024 * 1_024] {
        let data = vec![0xABu8; size];
        let meta = PvMetadata {
            original_name: "bench.bin".to_string(),
            original_size: size as u64,
            created_ts: 0,
            modified_ts: 0,
            mime_type: "application/octet-stream".to_string(),
        };

        group.bench_with_input(BenchmarkId::new("write", size), &size, |b, _| {
            b.iter(|| {
                let mut buf = Vec::with_capacity(size + 512);
                write_pv(
                    &mut buf,
                    &key,
                    &meta,
                    data.as_slice(),
                    &JobControl::default(),
                )
                .unwrap();
                buf
            });
        });

        let mut pv_buf = Vec::new();
        write_pv(
            &mut pv_buf,
            &key,
            &meta,
            data.as_slice(),
            &JobControl::default(),
        )
        .unwrap();
        group.bench_with_input(BenchmarkId::new("read", size), &size, |b, _| {
            b.iter(|| read_pv(&mut Cursor::new(&pv_buf), &key).unwrap());
        });
    }
    group.finish();
}

// ── Argon2id (key derivation) — intentionally separate / slow ─────────────────

fn bench_argon2id(c: &mut Criterion) {
    // Production-cost params: m=64MB, t=3, p=1
    let params = KdfParams {
        salt: vec![42u8; 32],
        m_cost: 65536,
        t_cost: 3,
        p_cost: 1,
    };
    c.bench_function("argon2id_production_params", |b| {
        b.iter(|| derive_key("benchmark_password_123!", &params).unwrap());
    });
}

criterion_group!(fast_benches, bench_aes_gcm, bench_pv_file);
criterion_group! {
    name = slow_benches;
    config = Criterion::default().sample_size(10);
    targets = bench_argon2id
}
criterion_main!(fast_benches, slow_benches);
