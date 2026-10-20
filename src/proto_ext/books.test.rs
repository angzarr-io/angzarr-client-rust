use super::*;
use crate::proto::{EventPage, Handling};

#[test]
fn next_sequence_follows_the_last_page_then_the_snapshot() {
    let page = |sequence| EventPage {
        sequence,
        ..Default::default()
    };
    let snapshot = Snapshot {
        sequence: 6,
        ..Default::default()
    };
    assert_eq!(calculate_next_sequence(&[page(3), page(4)], None), 5);
    assert_eq!(calculate_next_sequence(&[page(9)], Some(&snapshot)), 10);
    assert_eq!(calculate_next_sequence(&[], Some(&snapshot)), 7);
    assert_eq!(calculate_next_sequence(&[], None), 0);
}

#[test]
fn handling_reads_unset_as_absent_and_unspecified() {
    let book = CommandBook::default();
    assert_eq!(book.expected_sequence(), None);
    assert_eq!(book.conflict_resolution(), ConflictResolution::Unspecified);
    assert!(book.command_page().is_none());

    let book = CommandBook {
        page: Some(CommandPage::default()),
        handling: Some(Handling {
            expected_sequence: Some(4),
            conflict_resolution: ConflictResolution::Strict as i32,
        }),
        ..Default::default()
    };
    assert_eq!(book.expected_sequence(), Some(4));
    assert_eq!(book.conflict_resolution(), ConflictResolution::Strict);
    assert!(book.command_page().is_some());
}
