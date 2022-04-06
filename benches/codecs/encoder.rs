use bytes::{BufMut, BytesMut};
use codecs::{JsonSerializer, NewlineDelimitedEncoder};
use criterion::{
    criterion_group, measurement::WallTime, BatchSize, BenchmarkGroup, Criterion, SamplingMode,
    Throughput,
};
use std::time::Duration;
use tokio_util::codec::Encoder;
use vector::event::Event;
use vector_common::{btreemap, byte_size_of::ByteSizeOf};

#[derive(Debug, Clone)]
pub struct JsonLogSerializer;

impl Encoder<Event> for JsonLogSerializer {
    type Error = vector_core::Error;

    fn encode(&mut self, event: Event, buffer: &mut BytesMut) -> Result<(), Self::Error> {
        let writer = buffer.writer();
        let log = event.as_log();
        serde_json::to_writer(writer, log)?;
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct JsonLogVecSerializer;

impl Encoder<Event> for JsonLogVecSerializer {
    type Error = vector_core::Error;

    fn encode(&mut self, event: Event, buffer: &mut BytesMut) -> Result<(), Self::Error> {
        let log = event.as_log();
        let vec = serde_json::to_vec(log)?;
        buffer.put_slice(&vec);
        Ok(())
    }
}

fn encoder(c: &mut Criterion) {
    let mut group: BenchmarkGroup<WallTime> = c.benchmark_group("encoder");
    group.sampling_mode(SamplingMode::Auto);

    let input: Event = btreemap! {
        "key1" => "value1",
        "key2" => "value2",
        "key3" => "value3"
    }
    .into();

    group.throughput(Throughput::Bytes(input.size_of() as u64));
    group.bench_with_input("vector::sinks::util::encode_log", &(), |b, ()| {
        b.iter_batched(
            || vector::sinks::util::Encoding::Json.into(),
            |encoding| {
                vector::sinks::util::encode_log(input.clone(), &encoding).unwrap();
            },
            BatchSize::SmallInput,
        )
    });

    group.throughput(Throughput::Bytes(input.size_of() as u64));
    group.bench_with_input("JsonLogVecSerializer::encode", &(), |b, ()| {
        b.iter_batched(
            || JsonLogVecSerializer,
            |mut encoder| {
                let mut bytes = BytesMut::new();
                encoder.encode(input.clone(), &mut bytes).unwrap();
                bytes.put_u8(b'\n');
            },
            BatchSize::SmallInput,
        )
    });

    group.throughput(Throughput::Bytes(input.size_of() as u64));
    group.bench_with_input("JsonLogSerializer::encode", &(), |b, ()| {
        b.iter_batched(
            || JsonLogSerializer,
            |mut encoder| {
                let mut bytes = BytesMut::new();
                encoder.encode(input.clone(), &mut bytes).unwrap();
                bytes.put_u8(b'\n');
            },
            BatchSize::SmallInput,
        )
    });

    group.throughput(Throughput::Bytes(input.size_of() as u64));
    group.bench_with_input("codecs::JsonSerializer::encode", &(), |b, ()| {
        b.iter_batched(
            || JsonSerializer::new(),
            |mut encoder| {
                let mut bytes = BytesMut::new();
                encoder.encode(input.clone(), &mut bytes).unwrap();
                bytes.put_u8(b'\n');
            },
            BatchSize::SmallInput,
        )
    });

    group.throughput(Throughput::Bytes(input.size_of() as u64));
    group.bench_with_input("vector::codecs::Encoder::encode", &(), |b, ()| {
        b.iter_batched(
            || {
                vector::codecs::Encoder::new(
                    NewlineDelimitedEncoder::new().into(),
                    JsonSerializer::new().into(),
                )
            },
            |mut encoder| {
                let mut bytes = BytesMut::new();
                encoder.encode(input.clone(), &mut bytes).unwrap();
            },
            BatchSize::SmallInput,
        )
    });
}

criterion_group!(
    name = benches;
    config = Criterion::default()
        .warm_up_time(Duration::from_secs(5))
        .measurement_time(Duration::from_secs(120))
        // degree of noise to ignore in measurements, here 1%
        .noise_threshold(0.01)
        // likelihood of noise registering as difference, here 5%
        .significance_level(0.05)
        // likelihood of capturing the true runtime, here 95%
        .confidence_level(0.95)
        // total number of bootstrap resamples, higher is less noisy but slower
        .nresamples(100_000)
        // total samples to collect within the set measurement time
        .sample_size(150);
    targets = encoder
);
