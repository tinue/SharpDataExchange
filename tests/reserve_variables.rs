//! End-to-end byte-parity/round-trip coverage for Reserve Area (SDAR) and Variables
//! (SDAV) transfer: a binary payload behind a CE-158 header decodes to ASCII text via
//! `get_cmd::process_normal`, and that ASCII text re-encodes via
//! `put_cmd::build_put_bytes` back to byte-identical bytes.

#![cfg(feature = "serial")]

use sharpdx::header::{self, BuildHeader, FileType};
use sharpdx::pocket_device::PocketDevice;
use sharpdx::put_cmd::PutOptions;
use sharpdx::registry::{Device, Registry};
use sharpdx::{detect, reserve, variables};

fn put_opts(input_file: &str) -> PutOptions {
    PutOptions {
        device: None,
        port: None,
        format: None,
        start_address: None,
        run_address: None,
        raw: false,
        dry_run: false,
        flow_control: false,
        verbose: false,
        input_file: input_file.to_string(),
    }
}

#[test]
fn reserve_byte_parity_round_trip() {
    let reg = Registry::for_device(Device::Pc1500);
    let mut layout = reserve::ReserveLayout::default();
    layout.labels[0] = "MENU".to_string();
    layout.labels[1] = "MATH".to_string();
    layout.keys[0][0] = "PRINT".to_string();
    layout.keys[0][1] = "ABS(".to_string();
    layout.keys[1][5] = "HELLO".to_string();
    let payload = reserve::encode_payload(&layout, reg).unwrap();

    let mut original = header::build_header(BuildHeader {
        device: Device::Pc1500,
        file_type: FileType::Reserve,
        name: Some("RESFILE"),
        payload_len: payload.len(),
        start_addr: 0,
        run_addr: 0,
    });
    original.extend_from_slice(&payload);

    // get: binary -> ASCII
    let content = detect::detect(&original);
    assert_eq!(content, detect::Content::Ce158Reserve);
    let ascii_text = {
        // process_normal is private to get_cmd; reproduce its Reserve branch via the
        // public building blocks it uses, since only the module-level unit test can
        // call process_normal directly.
        let h = header::find(&original).unwrap();
        let payload = &original[h.payload_start()..h.payload_start() + h.length];
        let mut layout = reserve::decode_payload(payload, reg).unwrap();
        layout.filename = h.filename.clone();
        reserve::to_ascii(&layout)
    };

    // put: ASCII -> binary, byte-identical to the original
    let content = detect::detect(ascii_text.as_bytes());
    assert_eq!(content, detect::Content::Ce158Reserve);
    let (rebuilt, _header_len) = sharpdx::put_cmd::build_put_bytes(
        ascii_text.as_bytes(),
        None,
        content,
        &put_opts("resfile.sdar"),
        PocketDevice::Pc1500,
    )
    .unwrap();
    assert_eq!(rebuilt, original);
}

#[test]
fn variables_byte_parity_round_trip() {
    let values = vec![
        variables::VarValue::NumericScalar("3.14159265".to_string()),
        variables::VarValue::StringScalar(b"Hi there!".to_vec()),
        variables::VarValue::NumericArray(vec!["1".to_string(), "2".to_string(), "3".to_string()]),
        variables::VarValue::StringArray {
            max_len: 16,
            elements: vec![b"one".to_vec(), b"two".to_vec()],
        },
    ];
    let payload = variables::encode_payload(&values).unwrap();

    let mut original = header::build_header(BuildHeader {
        device: Device::Pc1500,
        file_type: FileType::Variables,
        name: Some("VARFILE"),
        payload_len: payload.len(),
        start_addr: 0,
        run_addr: 0,
    });
    original.extend_from_slice(&payload);

    let content = detect::detect(&original);
    assert_eq!(content, detect::Content::Ce158Variables);

    let ascii_text = {
        let h = header::find(&original).unwrap();
        // Variables' header length field is unreliable -- read to end of buffer.
        let payload = &original[h.payload_start()..];
        let mut file = variables::decode_payload(payload).unwrap();
        file.filename = h.filename.clone();
        variables::to_ascii(&file)
    };

    let content = detect::detect(ascii_text.as_bytes());
    assert_eq!(content, detect::Content::Ce158Variables);
    let (rebuilt, _header_len) = sharpdx::put_cmd::build_put_bytes(
        ascii_text.as_bytes(),
        None,
        content,
        &put_opts("varfile.sdav"),
        PocketDevice::Pc1500,
    )
    .unwrap();
    assert_eq!(rebuilt, original);
}
