//! Minimal end-to-end demo of buffa-generated message types produced by the
//! published `buf.build/anthropics/buffa` BSR remote plugin.
//!
//! Run with `cargo run` from this directory. To regenerate `src/gen/` after
//! editing `proto/`, run `buf generate` (or `task gen-bsr-quickstart-example`
//! from the repository root).

mod gen;

use buffa::{Message, MessageView};
use gen::example::v1::{
    greeting::{Recipient, RecipientView},
    Greeting, GreetingView, Mood,
};

fn main() {
    // 1. Construct a message. Generated structs implement `Default`, so you
    //    only need to set the fields you care about. Enum variants keep the
    //    proto name verbatim (`MOOD_FRIENDLY`); `EnumValue<E>` wraps an open
    //    enum, and `.into()` converts from the bare variant.
    let greeting = Greeting {
        text: "Hello from buffa!".into(),
        at: buffa_types::google::protobuf::Timestamp {
            seconds: 1_700_000_000,
            nanos: 0,
            ..Default::default()
        }
        .into(),
        mood: Mood::MOOD_FRIENDLY.into(),
        recipient: Some(Recipient::Name("Buf".into())),
        tags: vec!["demo".into(), "bsr".into()],
        ..Default::default()
    };

    // 2. Binary protobuf round-trip. `encode_to_vec` and `decode_from_slice`
    //    come from the `buffa::Message` trait.
    let wire = greeting.encode_to_vec();
    println!("encoded {} bytes", wire.len());
    let decoded = Greeting::decode_from_slice(&wire).expect("decode failed");
    assert_eq!(decoded, greeting);

    // 3. Zero-copy view: read fields straight from the wire bytes without
    //    materialising owned `String`/`Vec` allocations. The view type is
    //    re-exported at the package root alongside the owned struct. View
    //    fields are direct (`&str`, `RepeatedView<&str>`, …), not getters.
    //    Oneof view enums (`RecipientView`) live in a `<message_name>` module
    //    next to the owned `Recipient` enum.
    let view = GreetingView::decode_view(&wire).expect("decode_view failed");
    println!("view text = {:?}", view.text);
    println!("view tags = {:?}", view.tags.iter().collect::<Vec<_>>());
    match view.recipient {
        Some(RecipientView::Name(name)) => println!("view recipient = name {name:?}"),
        Some(RecipientView::Everyone(true)) => println!("view recipient = everyone"),
        _ => println!("view recipient = unset"),
    }

    // 4. Proto3 JSON. Generated types implement `serde::Serialize` and
    //    `serde::Deserialize` because the plugin runs with `json=true`.
    let json = serde_json::to_string_pretty(&greeting).expect("to JSON");
    println!("{json}");
    let from_json: Greeting = serde_json::from_str(&json).expect("from JSON");
    assert_eq!(from_json, greeting);
}
