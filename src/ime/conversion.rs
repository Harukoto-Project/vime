/// One clause (文節) of a conversion with the candidates the user can pick from.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Segment {
    pub reading: String,
    pub candidates: Vec<String>,
    pub selected: usize,
}

impl Segment {
    pub fn text(&self) -> &str {
        &self.candidates[self.selected]
    }
}

/// A converted preedit made of clauses. One clause has the focus; Space cycles
/// its candidates and the arrows move the focus, like a desktop IME.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Conversion {
    segments: Vec<Segment>,
    focus: usize,
}

impl Conversion {
    pub fn new(segments: Vec<Segment>) -> Option<Self> {
        segments
            .iter()
            .all(|segment| segment.selected < segment.candidates.len())
            .then_some(())?;
        (!segments.is_empty()).then_some(Self { segments, focus: 0 })
    }

    pub fn segments(&self) -> &[Segment] {
        &self.segments
    }

    pub fn focus(&self) -> usize {
        self.focus
    }

    pub fn focused(&self) -> &Segment {
        &self.segments[self.focus]
    }

    pub fn text(&self) -> String {
        self.segments.iter().map(Segment::text).collect()
    }

    pub fn is_last(&self) -> bool {
        self.focus + 1 == self.segments.len()
    }

    pub fn move_focus(&mut self, step: i32) {
        self.focus = (self.focus as i32 + step).clamp(0, self.segments.len() as i32 - 1) as usize;
    }

    /// Chooses the next candidate of the focused clause, wrapping around.
    pub fn next_candidate(&mut self) {
        let segment = &mut self.segments[self.focus];
        segment.selected = (segment.selected + 1) % segment.candidates.len();
    }

    /// Chooses `candidate` for the focused clause.
    pub fn select(&mut self, candidate: &str) {
        let segment = &mut self.segments[self.focus];
        segment.selected = match segment
            .candidates
            .iter()
            .position(|value| value == candidate)
        {
            Some(index) => index,
            None => {
                segment.candidates.push(candidate.to_owned());
                segment.candidates.len() - 1
            }
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segment(reading: &str, candidates: &[&str]) -> Segment {
        Segment {
            reading: reading.into(),
            candidates: candidates.iter().map(|value| (*value).into()).collect(),
            selected: 0,
        }
    }

    #[test]
    fn cycles_candidates_and_moves_the_focus_inside_the_bounds() {
        let mut conversion = Conversion::new(vec![
            segment("はし", &["橋", "箸", "端"]),
            segment("を", &["を"]),
        ])
        .unwrap();
        conversion.next_candidate();
        conversion.next_candidate();
        conversion.next_candidate();
        assert_eq!(conversion.text(), "橋を");
        conversion.next_candidate();
        assert_eq!(conversion.text(), "箸を");
        conversion.move_focus(-1);
        assert_eq!(conversion.focus(), 0);
        conversion.move_focus(5);
        assert_eq!((conversion.focus(), conversion.is_last()), (1, true));
        conversion.select("ヲ");
        assert_eq!(conversion.text(), "箸ヲ");
    }

    #[test]
    fn rejects_empty_or_invalid_conversions() {
        assert!(Conversion::new(Vec::new()).is_none());
        assert!(Conversion::new(vec![segment("あ", &[])]).is_none());
    }
}
