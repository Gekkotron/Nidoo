use super::*;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::time::{Duration, Instant};

use nidoo_voice::{BoardMsg, HubMsg, Key, WakeKind, decode_board, encode_hub};

const SID: &str = "0123456789ab";
const HUB: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(192, 0, 2, 10), 4960);

fn key() -> Key {
    Key::from_bytes(std::array::from_fn(|i| i as u8))
}

fn link() -> VoiceLink {
    VoiceLink::new(key(), SID.to_string())
}

fn hello(nonce: u64, hub_ctr: u64) -> Vec<u8> {
    encode_hub(
        &key(),
        SID,
        &HubMsg::Hello {
            nonce,
            hub_ctr,
            hub: HUB,
        },
    )
}

fn board(bytes: &[u8]) -> BoardMsg {
    let (sid, msg) = decode_board(&key(), bytes).unwrap();
    assert_eq!(sid, SID);
    msg
}

#[test]
fn no_wake_before_hello() {
    let mut l = link();
    assert_eq!(l.open(WakeKind::WakeWord, 0, Instant::now()), None);
    assert!(!l.window_open());
    assert!(l.audio(&[1, 2, 3]).is_empty());
}

#[test]
fn wake_echoes_latest_nonce_and_sessions_rise() {
    let t = Instant::now();
    let mut l = link();
    assert_eq!(l.on_datagram(&hello(5, 10), t), Some(VoiceEvent::Greeted));
    assert_eq!(l.on_datagram(&hello(9, 11), t), Some(VoiceEvent::Greeted));
    let wake = l.open(WakeKind::WakeWord, 0, t).unwrap();
    assert_eq!(
        board(&wake),
        BoardMsg::Wake {
            nonce: 9,
            session: 1,
            kind: WakeKind::WakeWord,
            listen_ctr: 0
        }
    );
    assert_eq!(l.open(WakeKind::WakeWord, 0, t), None, "already open");
    assert_eq!(
        board(&l.close_local().unwrap()),
        BoardMsg::End { session: 1 }
    );
    assert_eq!(l.close_local(), None, "nothing left to close");
    let wake = l.open(WakeKind::WakeWord, 0, t).unwrap();
    assert!(matches!(board(&wake), BoardMsg::Wake { session: 2, .. }));
}

#[test]
fn streams_to_hello_address_not_source() {
    let mut l = link();
    assert_eq!(l.hub(), None);
    l.on_datagram(&hello(1, 1), Instant::now());
    assert_eq!(l.hub(), Some(HUB));
}

#[test]
fn audio_splits_at_640_with_rising_seq() {
    let t = Instant::now();
    let mut l = link();
    l.on_datagram(&hello(1, 1), t);
    l.open(WakeKind::WakeWord, 0, t).unwrap();
    let samples: Vec<i16> = (0..1500).map(|i| i as i16).collect();
    let out: Vec<BoardMsg> = l.audio(&samples).iter().map(|d| board(d)).collect();
    let shape: Vec<(u32, u32, usize)> = out
        .iter()
        .map(|m| match m {
            BoardMsg::Audio {
                session,
                seq,
                samples,
            } => (*session, *seq, samples.len()),
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(shape, vec![(1, 1, 640), (1, 2, 640), (1, 3, 220)]);
    let BoardMsg::Audio { samples: last, .. } = &out[2] else {
        unreachable!()
    };
    assert_eq!(last[0], 1280);
    // A new window starts again at seq 1.
    l.close_local();
    l.open(WakeKind::WakeWord, 0, t).unwrap();
    assert!(matches!(
        board(&l.audio(&[0])[0]),
        BoardMsg::Audio {
            session: 2,
            seq: 1,
            ..
        }
    ));
}

#[test]
fn stop_closes_only_its_session() {
    let t = Instant::now();
    let mut l = link();
    l.on_datagram(&hello(1, 1), t);
    l.open(WakeKind::WakeWord, 0, t).unwrap();
    let other = encode_hub(
        &key(),
        SID,
        &HubMsg::Stop {
            hub_ctr: 2,
            session: 7,
        },
    );
    assert_eq!(l.on_datagram(&other, t), None);
    assert!(l.window_open());
    let ours = encode_hub(
        &key(),
        SID,
        &HubMsg::Stop {
            hub_ctr: 3,
            session: 1,
        },
    );
    assert_eq!(l.on_datagram(&ours, t), Some(VoiceEvent::Stop));
    assert!(!l.window_open());
    assert!(l.audio(&[1]).is_empty());
    assert_eq!(
        l.tick(t + Duration::from_secs(9)),
        None,
        "no End after a Stop"
    );
}

#[test]
fn window_times_out_with_end() {
    let t = Instant::now();
    let mut l = link();
    l.on_datagram(&hello(1, 1), t);
    l.open(WakeKind::WakeWord, 0, t).unwrap();
    assert_eq!(l.tick(t + Duration::from_millis(7_999)), None);
    let end = l.tick(t + Duration::from_secs(8)).unwrap();
    assert_eq!(board(&end), BoardMsg::End { session: 1 });
    assert!(!l.window_open());

    let listen = encode_hub(&key(), SID, &HubMsg::Listen { hub_ctr: 2 });
    l.on_datagram(&listen, t);
    l.open(WakeKind::FollowUp, 2, t).unwrap();
    assert_eq!(l.tick(t + Duration::from_millis(5_999)), None);
    let end = l.tick(t + Duration::from_secs(6)).unwrap();
    assert_eq!(board(&end), BoardMsg::End { session: 2 });
}

#[test]
fn listen_opens_follow_up() {
    let t = Instant::now();
    let mut l = link();
    l.on_datagram(&hello(4, 1), t);
    let listen = encode_hub(&key(), SID, &HubMsg::Listen { hub_ctr: 77 });
    assert_eq!(
        l.on_datagram(&listen, t),
        Some(VoiceEvent::Listen { listen_ctr: 77 })
    );
    let wake = l.open(WakeKind::FollowUp, 77, t).unwrap();
    assert_eq!(
        board(&wake),
        BoardMsg::Wake {
            nonce: 4,
            session: 1,
            kind: WakeKind::FollowUp,
            listen_ctr: 77
        }
    );
    // A Listen while a window is open is not a new follow-up.
    let again = encode_hub(&key(), SID, &HubMsg::Listen { hub_ctr: 78 });
    assert_eq!(l.on_datagram(&again, t), None);
}

#[test]
fn ignores_wrong_key_other_sid_and_stale_ctr() {
    let t = Instant::now();
    let mut l = link();
    let wrong_key = encode_hub(
        &Key::from_bytes([9; 32]),
        SID,
        &HubMsg::Hello {
            nonce: 1,
            hub_ctr: 1,
            hub: HUB,
        },
    );
    assert_eq!(l.on_datagram(&wrong_key, t), None);
    let other_sid = encode_hub(
        &key(),
        "ffffffffffff",
        &HubMsg::Hello {
            nonce: 1,
            hub_ctr: 1,
            hub: HUB,
        },
    );
    assert_eq!(l.on_datagram(&other_sid, t), None);
    assert_eq!(l.on_datagram(b"junk", t), None);
    assert!(!l.greeted(t));
    assert_eq!(l.hub(), None);

    assert_eq!(l.on_datagram(&hello(5, 10), t), Some(VoiceEvent::Greeted));
    assert_eq!(l.on_datagram(&hello(6, 10), t), None, "same ctr");
    assert_eq!(l.on_datagram(&hello(6, 9), t), None, "older ctr");
    let wake = l.open(WakeKind::WakeWord, 0, t).unwrap();
    assert!(matches!(board(&wake), BoardMsg::Wake { nonce: 5, .. }));
    let stale_stop = encode_hub(
        &key(),
        SID,
        &HubMsg::Stop {
            hub_ctr: 3,
            session: 1,
        },
    );
    assert_eq!(l.on_datagram(&stale_stop, t), None);
    assert!(l.window_open());
}

#[test]
fn greeted_expires_after_15s_and_comes_back() {
    let t = Instant::now();
    let mut l = link();
    assert!(!l.greeted(t));
    l.on_datagram(&hello(1, 1), t);
    assert!(l.greeted(t + Duration::from_millis(14_999)));
    assert!(!l.greeted(t + Duration::from_secs(15)));
    assert_eq!(
        l.open(WakeKind::WakeWord, 0, t + Duration::from_secs(15)),
        None
    );
    let later = t + Duration::from_secs(40);
    l.on_datagram(&hello(2, 2), later);
    assert!(l.greeted(later));
    assert!(l.open(WakeKind::WakeWord, 0, later).is_some());
}
