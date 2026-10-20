use super::*;
use crate::proto_ext::EventPageExt;

fn any() -> Any {
    Any {
        type_url: "t".into(),
        value: vec![],
    }
}

#[test]
fn make_cover_sets_root() {
    let root = [1u8; 16];
    let cover = make_cover("player", root);
    assert_eq!(cover.domain, "player");
    assert_eq!(cover.root.unwrap().value.as_ref(), &root[..]);
}

#[test]
fn make_event_book_defaults_next_sequence() {
    let cover = make_cover("x", [0u8; 16]);
    let pages = vec![make_event_page(0, any()), make_event_page(1, any())];
    let book = make_event_book(cover, pages, None);
    assert_eq!(book.pages.len(), 2);
    assert_eq!(book.next_sequence, 2);
}

#[test]
fn make_event_page_sets_sequence_and_recorded_at() {
    let page = make_event_page(7, any());
    assert_eq!(page.sequence_num(), 7);
    assert!(page.meta.and_then(|m| m.recorded_at).is_some());
}

#[test]
fn make_command_book_carries_the_command() {
    let book = make_command_book(make_cover("x", [0u8; 16]), any(), None);
    let page = book.page.expect("one command page");
    assert_eq!(angzarr_router::command_payload(&page), Some(&any()));
}

#[test]
fn make_command_book_sends_handling_only_for_a_sequence() {
    let unset = make_command_book(make_cover("x", [0u8; 16]), any(), None);
    assert!(unset.handling.is_none());
    let set = make_command_book(make_cover("x", [0u8; 16]), any(), Some(4));
    assert_eq!(set.handling.and_then(|h| h.expected_sequence), Some(4));
}
