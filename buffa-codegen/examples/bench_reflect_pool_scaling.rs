//! Scaling benchmark comparing two ways to embed `fds_bytes` as
//! `FILE_DESCRIPTOR_SET_BYTES: &[u8]` in `reflect_pool_module`: one
//! `proc_macro2` token per byte, vs. a single byte-string literal. Not
//! wired into `task bench` — `benchmarks/buffa`'s criterion suite covers
//! message encode/decode performance, not codegen-time cost.
//!
//! Builds synthetic, self-contained `FileDescriptorProto` sets (no external
//! `.proto` files, no `protoc`) at increasing file counts and times both
//! approaches, to show how each scales with the number of files (and thus
//! the encoded `FileDescriptorSet` size).
//!
//! Run with: `cargo run --release --example bench_reflect_pool_scaling -p buffa-codegen`

use std::time::Instant;

use buffa::Message;
use buffa_codegen::generated::descriptor::{
    DescriptorProto, FieldDescriptorProto, FileDescriptorProto, FileDescriptorSet,
};
use proc_macro2::TokenStream;
use quote::quote;

/// One independent file per synthetic package: a name, a package, and one
/// message with a couple of scalar fields — enough to produce a realistic,
/// non-trivial per-file descriptor size without needing real `.proto` input.
fn synthetic_files(count: usize) -> Vec<FileDescriptorProto> {
    (0..count)
        .map(|i| FileDescriptorProto {
            name: Some(format!("bench/pkg{i}/message.proto")),
            package: Some(format!("bench.pkg{i}")),
            syntax: Some("proto3".to_string()),
            message_type: vec![DescriptorProto {
                name: Some("Message".to_string()),
                field: vec![
                    FieldDescriptorProto {
                        name: Some("id".to_string()),
                        number: Some(1),
                        ..Default::default()
                    },
                    FieldDescriptorProto {
                        name: Some("label".to_string()),
                        number: Some(2),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }],
            ..Default::default()
        })
        .collect()
}

fn encode_fds(files: &[FileDescriptorProto]) -> Vec<u8> {
    FileDescriptorSet {
        file: files.to_vec(),
        ..Default::default()
    }
    .encode_to_vec()
}

/// One `proc_macro2` token per byte.
fn old_per_byte_tokens(fds_bytes: &[u8]) -> TokenStream {
    let byte_literals = fds_bytes.iter().map(|b| quote! { #b });
    quote! { pub const FILE_DESCRIPTOR_SET_BYTES: &[u8] = &[#(#byte_literals),*]; }
}

/// One token for the whole blob, regardless of its length.
fn new_byte_string_token(fds_bytes: &[u8]) -> TokenStream {
    let lit = proc_macro2::Literal::byte_string(fds_bytes);
    quote! { pub const FILE_DESCRIPTOR_SET_BYTES: &[u8] = #lit; }
}

/// Recursively counts every token, including ones nested inside delimited
/// groups (`(...)`, `[...]`, `{...}`) — a plain `TokenStream::into_iter()`
/// only counts the outermost tokens and treats each group as a single
/// opaque token, which would hide exactly the blowup this benchmark exists
/// to measure (the per-byte literals live inside a `[...]` group).
fn count_tokens_recursive(ts: TokenStream) -> usize {
    ts.into_iter()
        .map(|tt| match tt {
            proc_macro2::TokenTree::Group(g) => 1 + count_tokens_recursive(g.stream()),
            _ => 1,
        })
        .sum()
}

fn main() {
    println!(
        "{:>8} | {:>14} | {:>12} | {:>10} | {:>12} | {:>10}",
        "files", "fds_bytes", "old_tokens", "old_time", "new_tokens", "new_time"
    );
    println!("{}", "-".repeat(80));

    for &count in &[1usize, 10, 100, 1_000, 5_000, 10_000] {
        let files = synthetic_files(count);
        let fds_bytes = encode_fds(&files);

        let t0 = Instant::now();
        let old = old_per_byte_tokens(&fds_bytes);
        let old_time = t0.elapsed();
        let old_tokens = count_tokens_recursive(old);

        let t0 = Instant::now();
        let new = new_byte_string_token(&fds_bytes);
        let new_time = t0.elapsed();
        let new_tokens = count_tokens_recursive(new);

        println!(
            "{count:>8} | {:>14} | {old_tokens:>12} | {:>10} | {new_tokens:>12} | {:>10}",
            fds_bytes.len(),
            format!("{:.3?}", old_time),
            format!("{:.3?}", new_time),
        );
    }
}
