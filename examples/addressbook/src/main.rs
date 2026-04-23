//! Address book CLI — demonstrates encoding, decoding, and working with
//! buffa-generated protobuf types.
//!
//! Usage:
//!   addressbook add <file>       Add a person interactively
//!   addressbook list <file>      List all contacts
//!   addressbook show <file> <id> Show details for a contact
//!   addressbook dump <file>      Print the address book in textproto

// `#[allow(deprecated)]` silences codegen-internal references to
// deprecated fields (here: `AddressOneof::FreeformAddress`, which
// build.rs marks `#[deprecated]`). Generated encoders/decoders match
// on every variant regardless of deprecation, so the warnings fire
// inside the generated module itself; we only want them in *our*
// code, not in generated code we don't control.
#[allow(deprecated)]
mod proto {
    include!(concat!(env!("OUT_DIR"), "/_include.rs"));
}

use buffa::{EnumValue, Message};
use proto::buffa::examples::addressbook::v1::__buffa::oneof::person::Address as AddressOneof;
use proto::buffa::examples::addressbook::v1::{
    person::{PhoneNumber, PhoneType},
    AddressBook, Person, StructuredAddress,
};
use std::io::{self, BufRead, Write};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("Usage: addressbook <add|list|show|dump> <file> [id]");
        std::process::exit(1);
    }

    let command = &args[1];
    let file_path = &args[2];

    match command.as_str() {
        "add" => cmd_add(file_path),
        "list" => cmd_list(file_path),
        "dump" => cmd_dump(file_path),
        "show" => {
            let id: i32 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or_else(|| {
                eprintln!("Usage: addressbook show <file> <id>");
                std::process::exit(1);
            });
            cmd_show(file_path, id);
        }
        _ => {
            eprintln!("Unknown command: {command}");
            std::process::exit(1);
        }
    }
}

/// Load an address book from a file, or return an empty one.
fn load_address_book(path: &str) -> AddressBook {
    match std::fs::read(path) {
        Ok(data) => AddressBook::decode_from_slice(&data).unwrap_or_else(|e| {
            eprintln!("Warning: failed to decode {path}: {e}");
            AddressBook::default()
        }),
        Err(_) => AddressBook::default(),
    }
}

/// Save an address book to a file.
fn save_address_book(path: &str, book: &AddressBook) {
    let data = book.encode_to_vec();
    std::fs::write(path, &data).unwrap_or_else(|e| {
        eprintln!("Error writing {path}: {e}");
        std::process::exit(1);
    });
    println!("Saved ({} bytes).", data.len());
}

/// Prompt the user for input with a label.
fn prompt(label: &str) -> String {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    print!("{label}: ");
    stdout.flush().unwrap();
    let mut line = String::new();
    stdin.lock().read_line(&mut line).unwrap();
    line.trim().to_string()
}

fn cmd_add(file_path: &str) {
    let mut book = load_address_book(file_path);

    let name = prompt("Name");
    let id: i32 = prompt("ID (integer)")
        .parse()
        .expect("ID must be an integer");
    let email = prompt("Email");

    let mut phones = Vec::new();
    loop {
        let number = prompt("Phone number (empty to finish)");
        if number.is_empty() {
            break;
        }
        let type_str = prompt("  Type (mobile/home/work)");
        let phone_type = match type_str.to_lowercase().as_str() {
            "mobile" => PhoneType::PHONE_TYPE_MOBILE,
            "home" => PhoneType::PHONE_TYPE_HOME,
            "work" => PhoneType::PHONE_TYPE_WORK,
            _ => PhoneType::PHONE_TYPE_UNSPECIFIED,
        };
        phones.push(PhoneNumber {
            number,
            r#type: EnumValue::Known(phone_type),
            ..Default::default()
        });
    }

    // Write path: the `freeform_address` variant is marked `#[deprecated]`
    // via a `field_attribute` in build.rs, so we don't offer it on new
    // entries — this is the typical migration pattern. Existing records
    // that already use it are still read correctly by `cmd_show` below.
    let address_choice = prompt("Address (type 'y' for structured, empty to skip)");
    let address = if matches!(address_choice.to_lowercase().as_str(), "y" | "yes" | "s") {
        let street = prompt("  Street");
        let city = prompt("  City");
        let state = prompt("  State");
        let zip_code = prompt("  Zip code");
        let country = prompt("  Country");
        Some(AddressOneof::StructuredAddress(Box::new(
            StructuredAddress {
                street,
                city,
                state,
                zip_code,
                country,
                ..Default::default()
            },
        )))
    } else {
        None
    };

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap();

    let person = Person {
        name,
        id,
        email,
        phones,
        last_updated: buffa::MessageField::some(buffa_types::google::protobuf::Timestamp {
            seconds: now.as_secs() as i64,
            nanos: now.subsec_nanos() as i32,
            ..Default::default()
        }),
        address,
        ..Default::default()
    };

    book.people.push(person);
    save_address_book(file_path, &book);
}

/// Print the entire address book in textproto — the human-readable debug
/// format. Useful for diffing in tests or eyeballing the binary file's
/// contents without a hex editor.
fn cmd_dump(file_path: &str) {
    let book = load_address_book(file_path);
    print!("{}", buffa::text::encode_to_string_pretty(&book));
}

fn cmd_list(file_path: &str) {
    let book = load_address_book(file_path);
    if book.people.is_empty() {
        println!("Address book is empty.");
        return;
    }
    for person in &book.people {
        let phone_count = person.phones.len();
        println!(
            "  #{}: {} <{}> ({} phone{})",
            person.id,
            person.name,
            person.email,
            phone_count,
            if phone_count == 1 { "" } else { "s" }
        );
    }
}

fn cmd_show(file_path: &str, id: i32) {
    let book = load_address_book(file_path);
    let person = book.people.iter().find(|p| p.id == id);
    let Some(person) = person else {
        println!("No contact with ID {id}.");
        return;
    };

    println!("Name:  {}", person.name);
    println!("ID:    {}", person.id);
    println!("Email: {}", person.email);

    if let Some(ts) = person.last_updated.as_option() {
        println!("Updated: {}s {}ns", ts.seconds, ts.nanos);
    }

    for phone in &person.phones {
        let type_name = match &phone.r#type {
            EnumValue::Known(PhoneType::PHONE_TYPE_MOBILE) => "mobile",
            EnumValue::Known(PhoneType::PHONE_TYPE_HOME) => "home",
            EnumValue::Known(PhoneType::PHONE_TYPE_WORK) => "work",
            _ => "unknown",
        };
        println!("Phone: {} ({})", phone.number, type_name);
    }

    // Read path: read both variants including the deprecated one for
    // back-compat with address books written before `freeform_address`
    // was deprecated. `#[allow(deprecated)]` is scoped to this match so
    // warnings still fire on any accidental *writes* elsewhere.
    #[allow(deprecated)]
    match &person.address {
        Some(AddressOneof::StructuredAddress(addr)) => {
            println!("Address:");
            println!("  {}", addr.street);
            println!("  {}, {} {}", addr.city, addr.state, addr.zip_code);
            println!("  {}", addr.country);
        }
        Some(AddressOneof::FreeformAddress(addr)) => {
            println!("Address: {addr} (legacy freeform format)");
        }
        None => {}
    }
}
