//! `generate()` must produce byte-identical output whether
//! `CodeGenConfig::shared_corpus_context` is
//! unset (recomputed fresh, today's behavior) or precomputed once via
//! [`crate::SharedCorpusContext::new`] and reused — the whole point
//! is that this is a pure caching layer with no observable effect on output.

use super::*;
use crate::generated::descriptor::field_descriptor_proto::{Label, Type};
use crate::generated::descriptor::source_code_info::Location;
use crate::generated::descriptor::{
    DescriptorProto, FieldDescriptorProto, OneofDescriptorProto, SourceCodeInfo,
};

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

    // Comments on a message and on a field, so the cached comment map is
    // exercised end to end (they surface as doc attributes in the output).
    let mut leaf_sci = SourceCodeInfo::default();
    leaf_sci
        .location
        .push(location(vec![4, 0], " The leaf message comment.\n"));
    leaf.source_code_info = leaf_sci.into();
    let mut main_sci = SourceCodeInfo::default();
    main_sci.location.push(location(
        vec![4, 1, 2, 0],
        " The inline leaf field comment.\n",
    ));
    main.source_code_info = main_sci.into();

    vec![leaf, main]
}

fn location(path: Vec<i32>, leading: &str) -> Location {
    Location {
        path,
        leading_comments: Some(leading.to_string()),
        ..Default::default()
    }
}

/// Assert two `generate()` outputs are byte-identical, order-independent.
fn assert_same_output(mut a: Vec<GeneratedFile>, mut b: Vec<GeneratedFile>, label: &str) {
    assert_eq!(a.len(), b.len(), "{label}: same number of generated files");
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

    config.shared_corpus_context = Some(crate::SharedCorpusContext::new(&files, &config));
    let with_shared =
        crate::generate(&files, &files_to_generate, &config).expect("shared-context generate");

    // Sanity: the corpus actually exercised what the shared context
    // resolves. `Inline` is the default pointer repr, so the non-recursive
    // `Holder.leaf` is stored inline while the self-recursive
    // `SelfRef.child` is demoted to the indirected form — both decisions
    // come from the cached `inlined_message_fields` set.
    let all = joined(&with_shared);
    assert!(
        all.contains("::buffa::Inline<Leaf>"),
        "Holder.leaf must be stored inline: {all}"
    );
    assert!(
        !all.contains("::buffa::Inline<SelfRef>"),
        "SelfRef.child must not be inlined into itself: {all}"
    );
    assert!(
        all.contains("The leaf message comment.") && all.contains("The inline leaf field comment."),
        "the cached comment map must reach the output: {all}"
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

    let shared = crate::SharedCorpusContext::new(&files, &leaf_config);
    let mut leaf_config_shared = leaf_config;
    leaf_config_shared.shared_corpus_context = Some(shared.clone());
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

/// The shared context is live, not merely tolerated: a context built under
/// different oneof rules than the call's is refused rather than silently
/// resolving the corpus against the wrong rules, and so is one built from a
/// different corpus.
#[test]
fn mismatched_shared_context_is_rejected() {
    let files = test_files();
    let files_to_generate = vec!["leaf.proto".to_string(), "main.proto".to_string()];

    let no_rules = CodeGenConfig::default();
    let mut with_rules = oneof_config();
    with_rules.shared_corpus_context = Some(crate::SharedCorpusContext::new(&files, &no_rules));
    let err = crate::generate(&files, &files_to_generate, &with_rules)
        .expect_err("a context built under different oneof rules must be refused");
    assert!(
        matches!(
            err,
            CodeGenError::SharedCorpusContextMismatch {
                what: "unboxed_oneof_fields"
            }
        ),
        "{err}"
    );

    let mut other_corpus = oneof_config();
    other_corpus.shared_corpus_context =
        Some(crate::SharedCorpusContext::new(&files[..1], &other_corpus));
    let err = crate::generate(&files, &files_to_generate, &other_corpus)
        .expect_err("a context built from a different corpus must be refused");
    assert!(
        matches!(
            err,
            CodeGenError::SharedCorpusContextMismatch { what: "files" }
        ),
        "{err}"
    );

    let mut pointer_rules = oneof_config();
    pointer_rules.shared_corpus_context =
        Some(crate::SharedCorpusContext::new(&files, &pointer_rules));
    pointer_rules.pointer_fields = vec![(".".to_string(), PointerRepr::Box)];
    let err = crate::generate(&files, &files_to_generate, &pointer_rules)
        .expect_err("a context built under different pointer rules must be refused");
    assert!(
        matches!(
            err,
            CodeGenError::SharedCorpusContextMismatch {
                what: "pointer_fields"
            }
        ),
        "{err}"
    );
}

/// `SharedCorpusContext` is handed to parallel per-package `generate()`
/// calls, so it must stay `Send + Sync`; a `Debug` of it (and so of a
/// `CodeGenConfig` carrying one) prints sizes, not the corpus comment text.
#[test]
fn shared_context_is_send_sync_and_debug_prints_sizes() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<crate::SharedCorpusContext>();

    let files = test_files();
    let shared = crate::SharedCorpusContext::new(&files, &oneof_config());
    let debug = format!("{shared:?}");
    assert!(
        debug.contains("files: 2") && debug.contains("comments: 2"),
        "{debug}"
    );
    assert!(!debug.contains("leaf message comment"), "{debug}");
}
