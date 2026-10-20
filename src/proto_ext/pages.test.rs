use super::*;
use crate::proto::{command_page, Command, Cover, Fact};

fn any(url: &str, value: Vec<u8>) -> Any {
    Any {
        type_url: url.into(),
        value,
    }
}

#[test]
fn decode_typed_matches_by_full_name_whatever_the_prefix() {
    let value = prost::Message::encode_to_vec(&Cover {
        domain: "d".into(),
        ..Default::default()
    });
    for url in [
        "/io.angzarr.v1.Cover",
        "type.googleapis.com/io.angzarr.v1.Cover",
    ] {
        let ev = EventPage {
            payload: Some(any(url, value.clone())),
            ..Default::default()
        };
        assert_eq!(
            ev.decode_typed::<Cover>().map(|c| c.domain),
            Some("d".into())
        );
        let cmd = CommandPage {
            body: Some(command_page::Body::Command(Command {
                payload: Some(any(url, value.clone())),
            })),
            ..Default::default()
        };
        assert_eq!(
            cmd.decode_typed::<Cover>().map(|c| c.domain),
            Some("d".into())
        );
    }
    let other = EventPage {
        payload: Some(any("/io.angzarr.v1.Target", value)),
        ..Default::default()
    };
    assert!(other.decode_typed::<Cover>().is_none());
}

#[test]
fn page_accessors_read_the_payload() {
    let ev = EventPage {
        payload: Some(any("/a.B", vec![1, 2])),
        sequence: 7,
        ..Default::default()
    };
    assert_eq!(ev.sequence_num(), 7);
    assert_eq!(ev.type_url(), Some("/a.B"));
    assert_eq!(ev.payload(), Some(&[1u8, 2][..]));
    assert_eq!(EventPage::default().type_url(), None);
    assert_eq!(EventPage::default().payload(), None);
}

#[test]
fn command_accessors_read_the_command_arm_only() {
    let cmd = CommandPage {
        body: Some(command_page::Body::Command(Command {
            payload: Some(any("/a.Do", vec![3])),
        })),
        ..Default::default()
    };
    assert_eq!(cmd.type_url(), Some("/a.Do"));
    assert_eq!(cmd.payload(), Some(&[3u8][..]));
    let fact = CommandPage {
        body: Some(command_page::Body::Fact(Fact {
            payload: Some(any("/a.Done", vec![4])),
        })),
        ..Default::default()
    };
    assert_eq!(fact.type_url(), None);
    assert_eq!(fact.payload(), None);
}
