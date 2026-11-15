//! Every file in `fixtures/malformed` starts with `# expect: <line> <KindName>` and must fail to
//! parse at that line with that kind of error.

use std::fs;
use std::path::Path;

use gatt_profile::parse;

#[test]
fn every_malformed_file_fails_where_it_says() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/malformed");
    let mut count = 0;
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let text = fs::read_to_string(&path).unwrap();
        let expect = text
            .lines()
            .next()
            .and_then(|l| l.strip_prefix("# expect: "))
            .unwrap_or_else(|| panic!("{} has no expect line", path.display()));
        let (line, kind) = expect.split_once(' ').unwrap();
        let err = parse(&text).expect_err(&path.display().to_string());
        assert_eq!(
            err.line,
            line.parse::<usize>().unwrap(),
            "{}",
            path.display()
        );
        let got = format!("{:?}", err.kind);
        assert!(
            got.starts_with(kind),
            "{}: expected {kind}, got {got}",
            path.display()
        );
        count += 1;
    }
    assert!(count >= 20, "corpus shrank to {count} files");
}

#[test]
fn mutated_input_never_panics() {
    // A cheap fuzz: every prefix and every single-byte mutation of a valid file parses or errors.
    let src = include_str!("../fixtures/fff0_serial.gatt");
    for end in 0..src.len() {
        if src.is_char_boundary(end) {
            let _ = parse(&src[..end]);
        }
    }
    for i in 0..src.len() {
        let mut b = src.as_bytes().to_vec();
        b[i] = 0;
        let _ = parse(&String::from_utf8_lossy(&b));
    }
}
