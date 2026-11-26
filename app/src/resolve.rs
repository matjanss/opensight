//! Unscored hits resolved by hand (app TODO): a hole (the curtain saw a shot, no sound) or a
//! missed shot (one in the dead time) has no position. The shooter or range officer checks
//! the paper card, then places it on the card, says it went through the same hole as
//! another shot, or counts it a miss. It then becomes a numbered shot in its place in the
//! series (the later shots move up by one) and can be put back to "not scored".
//!
//! The series and the unscored hits are two lists (`SensorApp::series`, `unscored`): an
//! unscored hit carries how many series shots came before it. A resolved hit keeps the
//! sensor's kind (hole or missed), so `Shot::scored()` still tells the sensor's shots from
//! the ones placed by hand; how it was resolved is kept beside it (`How`).

use crate::card::Score;
use crate::protocol::{Kind, Shot};

/// A hit: (boot, seq).
pub type Key = (u32, u32);

pub fn key(s: &Shot) -> Key {
    (s.boot, s.seq)
}

/// How a hit was resolved by hand.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum How {
    /// Placed on the card by hand: its position is the shot's x, y.
    Placed,
    /// Through the same hole as this shot: its position and score copied.
    Same(Key),
    /// A miss: 0, not on the card.
    Miss,
}

/// A miss's score.
pub const MISS: Score = Score {
    ring: 0,
    tenths: 0,
    inner_ten: false,
};

/// Moves `unscored[j]` into the series in its place, as `shot` with `score`; the unscored
/// hits after it move up by one. Returns its index in the series.
pub fn resolve(
    series: &mut Vec<(Shot, Score)>,
    unscored: &mut Vec<(usize, Shot)>,
    j: usize,
    shot: Shot,
    score: Score,
) -> usize {
    let (at, _) = unscored.remove(j);
    series.insert(at, (shot, score));
    // Those before it in `unscored` came before it; those from `j` on came after it.
    for (after, _) in unscored[j..].iter_mut() {
        *after += 1;
    }
    at
}

/// Puts `series[i]` back among the unscored hits, in its place, with no position. Returns
/// the hit.
pub fn unresolve(series: &mut Vec<(Shot, Score)>, unscored: &mut Vec<(usize, Shot)>, i: usize) -> Shot {
    let (mut shot, _) = series.remove(i);
    shot.x = 0;
    shot.y = 0;
    // An unscored hit with `after <= i` came before series[i]; the others after it.
    let j = unscored.iter().take_while(|(after, _)| *after <= i).count();
    for (after, _) in unscored[j..].iter_mut() {
        *after -= 1;
    }
    unscored.insert(j, (i, shot));
    shot
}

/// The event log's line for a decision (`eventlog`): `manual` and the hit (boot, seq, its
/// kind), then `as=place x= y=`, `as=same same=BOOT:SEQ x= y=`, `as=miss` or `as=undo`,
/// with the shot's number in the series and its score. The last line for a hit holds.
/// No `rec n=`, `cfg id=` or `dip n=` in it, so the log readers pass over it.
pub fn log_line(hit: &Shot, how: Option<How>, number: Option<usize>, score: Option<Score>) -> String {
    let kind = match hit.kind {
        Kind::Hole => "hole",
        Kind::Missed => "missed",
        Kind::Shot => "shot",
    };
    let mut s = format!("manual boot={:08X} seq={} kind={kind}", hit.boot, hit.seq);
    let pos = format!(" x={:.2} y={:.2}", hit.x_mm(), hit.y_mm());
    match how {
        Some(How::Placed) => s += &format!(" as=place{pos}"),
        Some(How::Same((boot, seq))) => s += &format!(" as=same same={boot:08X}:{seq}{pos}"),
        Some(How::Miss) => s += " as=miss",
        None => s += " as=undo",
    }
    if let Some(n) = number {
        s += &format!(" shot={n}");
    }
    if let Some(sc) = score {
        s += &format!(" score={}.{} ring={}", sc.tenths / 10, sc.tenths % 10, sc.ring);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(seq: u32, kind: Kind) -> Shot {
        Shot {
            seq,
            boot: 0xB007,
            time_ms: 0,
            x: 0,
            y: 0,
            event: 0,
            cal: 0,
            sigma: 0,
            kind,
            flags: 0,
        }
    }

    fn score(ring: u8) -> Score {
        Score {
            ring,
            tenths: u16::from(ring) * 10,
            inner_ten: false,
        }
    }

    /// The firing order as seqs: the series with the unscored hits in their places.
    fn order(series: &[(Shot, Score)], unscored: &[(usize, Shot)]) -> Vec<(u32, bool)> {
        let mut out = Vec::new();
        for i in 0..=series.len() {
            out.extend(
                unscored
                    .iter()
                    .filter(|(a, _)| *a == i)
                    .map(|(_, s)| (s.seq, false)),
            );
            if let Some((s, _)) = series.get(i) {
                out.push((s.seq, true));
            }
        }
        out
    }

    /// Shots 1, 2, then holes 3 and 4, shot 5, missed 6, shot 7.
    type Lists = (Vec<(Shot, Score)>, Vec<(usize, Shot)>);

    fn fired() -> Lists {
        let mut series = Vec::new();
        let mut unscored = Vec::new();
        for (seq, kind) in [
            (1, Kind::Shot),
            (2, Kind::Shot),
            (3, Kind::Hole),
            (4, Kind::Hole),
            (5, Kind::Shot),
            (6, Kind::Missed),
            (7, Kind::Shot),
        ] {
            let s = hit(seq, kind);
            if s.scored() {
                series.push((s, score(9)));
            } else {
                unscored.push((series.len(), s));
            }
        }
        (series, unscored)
    }

    #[test]
    fn resolved_in_place_and_back() {
        let (mut series, mut unscored) = fired();
        let all = order(&series, &unscored);
        assert_eq!(all.iter().map(|o| o.0).collect::<Vec<_>>(), [1, 2, 3, 4, 5, 6, 7]);

        // Hole 4 first (the second of two in a row), then 6, then 3.
        let j = unscored.iter().position(|(_, s)| s.seq == 4).unwrap();
        let mut placed = unscored[j].1;
        placed.x = 123;
        assert_eq!(resolve(&mut series, &mut unscored, j, placed, score(8)), 2);
        assert_eq!(
            order(&series, &unscored),
            [
                (1, true),
                (2, true),
                (3, false),
                (4, true),
                (5, true),
                (6, false),
                (7, true)
            ]
        );
        let j = unscored.iter().position(|(_, s)| s.seq == 6).unwrap();
        let h = unscored[j].1;
        assert_eq!(resolve(&mut series, &mut unscored, j, h, MISS), 4);
        let j = unscored.iter().position(|(_, s)| s.seq == 3).unwrap();
        let h = unscored[j].1;
        assert_eq!(resolve(&mut series, &mut unscored, j, h, score(7)), 2);
        assert!(unscored.is_empty());
        let seqs: Vec<u32> = series.iter().map(|(s, _)| s.seq).collect();
        assert_eq!(seqs, [1, 2, 3, 4, 5, 6, 7]);
        assert_eq!(series[3].0.x, 123);
        assert_eq!(series[3].1.ring, 8);

        // Back to not scored: 4 (between two resolved ones), then 3, then 6.
        let back = unresolve(&mut series, &mut unscored, 3);
        assert_eq!((back.seq, back.x), (4, 0));
        assert_eq!(
            order(&series, &unscored),
            [
                (1, true),
                (2, true),
                (3, true),
                (4, false),
                (5, true),
                (6, true),
                (7, true)
            ]
        );
        unresolve(&mut series, &mut unscored, 2);
        let i = series.iter().position(|(s, _)| s.seq == 6).unwrap();
        unresolve(&mut series, &mut unscored, i);
        assert_eq!(order(&series, &unscored), all);
        assert_eq!(fired().1.iter().map(|(a, _)| *a).collect::<Vec<_>>(), [2, 2, 3]);
        assert_eq!(unscored.iter().map(|(a, _)| *a).collect::<Vec<_>>(), [2, 2, 3]);
    }

    #[test]
    fn before_the_first_shot() {
        let mut series = Vec::new();
        let mut unscored = vec![(0, hit(1, Kind::Hole))];
        series.push((hit(2, Kind::Shot), score(10)));
        let h = unscored[0].1;
        assert_eq!(resolve(&mut series, &mut unscored, 0, h, score(5)), 0);
        assert_eq!(series[0].0.seq, 1);
        unresolve(&mut series, &mut unscored, 0);
        assert_eq!(unscored[0].0, 0);
        assert_eq!(series[0].0.seq, 2);
    }

    #[test]
    fn log_lines() {
        let mut h = hit(17, Kind::Hole);
        h.boot = 0x1A2B3C4D;
        assert_eq!(
            log_line(&h, Some(How::Miss), Some(5), Some(MISS)),
            "manual boot=1A2B3C4D seq=17 kind=hole as=miss shot=5 score=0.0 ring=0"
        );
        h.x = -123;
        h.y = 456;
        let sc = Score {
            ring: 9,
            tenths: 92,
            inner_ten: false,
        };
        assert_eq!(
            log_line(&h, Some(How::Placed), Some(5), Some(sc)),
            "manual boot=1A2B3C4D seq=17 kind=hole as=place x=-1.23 y=4.56 shot=5 score=9.2 ring=9"
        );
        assert_eq!(
            log_line(&h, Some(How::Same((0xAB, 3))), Some(5), Some(sc)),
            "manual boot=1A2B3C4D seq=17 kind=hole as=same same=000000AB:3 x=-1.23 y=4.56 shot=5 score=9.2 ring=9"
        );
        h.x = 0;
        h.y = 0;
        let undo = log_line(&h, None, None, None);
        assert_eq!(undo, "manual boot=1A2B3C4D seq=17 kind=hole as=undo");
        assert!(!undo.contains("rec n=") && !undo.contains("cfg id=") && !undo.contains("dip n="));
    }
}
