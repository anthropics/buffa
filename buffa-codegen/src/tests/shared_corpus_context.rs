//! `generate()` must produce byte-identical output whether
//! `CodeGenConfig::shared_corpus_context` is
//! unset (recomputed fresh, today's behavior) or precomputed once via
//! [`crate::precompute_shared_corpus_context`] and reused — the whole point
//! is that this is a pure caching layer with no observable effect on output.

use super::*;
use crate::generated::descriptor::field_descriptor_proto::{Label, Type};
use crate::generated::descriptor::{DescriptorProto, FieldDescriptorProto, OneofDescriptorProto};

fn oneof_config() -> CodeGenConfig {
    CodeGenConfig {
        // Blanket rule so resolve_unboxed_variants does real (non-empty-early-return) work.
        unboxed_oneof_fields: vec![".".to_string()],
        ..Default::default()
    }
}

/// A corpus exercising every corpus-wide computation the shared context
/// caches: a non-recursive inline-eligible field (Leaf), a self-recursive
/// field that must stay boxed (SelfRef), and a oneof variant matched by the
/// blanket `unboxed_oneof_fields` rule (Holder.payload).
fn test_files() -> Vec<FileDescriptorProto> {
    let mut leaf = proto3_file("leaf.proto");
    leaf.message_type = vec![DescriptorProto {
        name: Some("Leaf".to_string()),
        field: vec![make_field(
            "value",
            1,
            Label::LABEL_OPTIONAL,
            Type::TYPE_INT32,
        )],
        ..Default::default()
    }];

    let mut main = proto3_file("main.proto");
    main.dependency = vec!["leaf.proto".to_string()];

    let mut inline_field = make_field("leaf", 1, Label::LABEL_OPTIONAL, Type::TYPE_MESSAGE);
    inline_field.type_name = Some(".Leaf".to_string());

    let mut self_field = make_field("child", 2, Label::LABEL_OPTIONAL, Type::TYPE_MESSAGE);
    self_field.type_name = Some(".SelfRef".to_string());

    let mut oneof_variant = FieldDescriptorProto {
        name: Some("payload".to_string()),
        number: Some(1),
        label: Some(Label::LABEL_OPTIONAL),
        r#type: Some(Type::TYPE_MESSAGE),
        type_name: Some(".Leaf".to_string()),
        oneof_index: Some(0),
        ..Default::default()
    };
    oneof_variant.json_name = Some("payload".to_string());

    main.message_type = vec![
        DescriptorProto {
            name: Some("SelfRef".to_string()),
            field: vec![self_field],
            ..Default::default()
        },
        DescriptorProto {
            name: Some("Holder".to_string()),
            field: vec![inline_field, oneof_variant],
            oneof_decl: vec![OneofDescriptorProto {
                name: Some("kind".to_string()),
                ..Default::default()
            }],
            ..Default::default()
        },
    ];

    vec![leaf, main]
}

/// Assert two `generate()` outputs are byte-identical, order-independent.
fn assert_same_output(a: Vec<GeneratedFile>, b: Vec<GeneratedFile>, label: &str) {
    assert_eq!(a.len(), b.len(), "{label}: same number of generated files");
    let mut a = a;
    let mut b = b;
    a.sort_by(|x, y| x.name.cmp(&y.name));
    b.sort_by(|x, y| x.name.cmp(&y.name));
    for (x, y) in a.iter().zip(b.iter()) {
        assert_eq!(x.name, y.name, "{label}");
        assert_eq!(
            x.content, y.content,
            "{label}: generated content for {} must be byte-identical",
            x.name
        );
    }
}

#[test]
fn shared_context_matches_fresh_computation() {
    let files = test_files();
    let files_to_generate = vec!["leaf.proto".to_string(), "main.proto".to_string()];
    let mut config = oneof_config();
    config.generate_views = true;

    let fresh = crate::generate(&files, &files_to_generate, &config).expect("fresh generate");

    let shared = crate::precompute_shared_corpus_context(&files, &config);
    config.shared_corpus_context = Some(std::sync::Arc::new(shared));
    let with_shared =
        crate::generate(&files, &files_to_generate, &config).expect("shared-context generate");

    // Sanity: the corpus actually exercised what we're testing. Owned
    // message fields use `MessageField<T>`, which is indirected internally
    // regardless of recursion, so self-recursion only forces a literal
    // `Box` in the *view* type (a borrowed `SelfRefView<'a>` referencing
    // itself would otherwise be infinite-size).
    let self_ref_view = with_shared
        .iter()
        .find(|f| f.content.contains("struct SelfRefView"))
        .expect("SelfRefView must be generated");
    assert!(
        self_ref_view.content.contains("Box"),
        "SelfRefView's self-referencing field must stay boxed: {}",
        self_ref_view.content
    );

    assert_same_output(fresh, with_shared, "single combined generate() call");
}

/// The production pattern this feature targets: one whole-corpus
/// `FileDescriptorSet`, one `SharedCorpusContext` precomputed from it, reused
/// across *separate* `generate()` calls that each emit a different subset of
/// files with a different `extern_paths` mapping for the files they don't
/// emit — `effective_extern_paths` is the one thing that legitimately varies
/// per call, and this must keep resolving correctly per call under sharing,
/// not just produce output that happens to be internally consistent.
#[test]
fn shared_context_across_separate_per_crate_calls() {
    let files = test_files();

    let leaf_call = vec!["leaf.proto".to_string()];
    let mut leaf_config = oneof_config();
    leaf_config.generate_views = true;

    let main_call = vec!["main.proto".to_string()];
    let mut main_config = oneof_config();
    main_config.generate_views = true;
    main_config.extern_paths = vec![(".Leaf".to_string(), "::leaf_crate::Leaf".to_string())];

    let fresh_leaf = crate::generate(&files, &leaf_call, &leaf_config).expect("fresh leaf crate");
    let fresh_main = crate::generate(&files, &main_call, &main_config).expect("fresh main crate");

    let shared = std::sync::Arc::new(crate::precompute_shared_corpus_context(
        &files,
        &leaf_config,
    ));
    let mut leaf_config_shared = leaf_config;
    leaf_config_shared.shared_corpus_context = Some(std::sync::Arc::clone(&shared));
    let mut main_config_shared = main_config;
    main_config_shared.shared_corpus_context = Some(shared);

    let shared_leaf =
        crate::generate(&files, &leaf_call, &leaf_config_shared).expect("shared leaf crate");
    let shared_main =
        crate::generate(&files, &main_call, &main_config_shared).expect("shared main crate");

    // Sanity: the main crate's per-call extern_paths must still take effect
    // under sharing — proving the shared context didn't bleed the leaf
    // crate's local generation into the main crate's call.
    let all_main = joined(&shared_main);
    assert!(
        all_main.contains("leaf_crate::Leaf"),
        "main crate's per-call extern_paths must resolve Leaf externally: {all_main}"
    );
    assert!(
        !all_main.contains("struct Leaf "),
        "main crate must not locally define Leaf when it's mapped extern: {all_main}"
    );

    assert_same_output(fresh_leaf, shared_leaf, "leaf crate");
    assert_same_output(fresh_main, shared_main, "main crate");
}
