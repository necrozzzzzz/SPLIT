use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PositionSnapshot {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub pitch: f64,
    pub yaw: f64,
    pub roll: f64,
}

impl PositionSnapshot {
    pub fn to_deadlock_command(&self) -> String {
        format!(
            "setpos_exact {} {} {};setang_exact {} {} {}",
            self.x, self.y, self.z, self.pitch, self.yaw, self.roll
        )
    }
}

fn parse_number(token: &str) -> Option<f64> {
    token
        .trim_matches(|c: char| matches!(c, '"' | '\'' | ',' | ';' | ')' | '('))
        .parse::<f64>()
        .ok()
}

fn parse_triplet_after(line: &str, markers: &[&str]) -> Option<[f64; 3]> {
    for marker in markers {
        let Some(index) = line.find(marker) else {
            continue;
        };

        let tail = &line[index + marker.len()..];
        let segment = tail.split(';').next().unwrap_or(tail);

        let mut values = segment
            .split_whitespace()
            .filter_map(parse_number);

        let a = values.next()?;
        let b = values.next()?;
        let c = values.next()?;

        return Some([a, b, c]);
    }

    None
}

pub fn parse_position_triplet(line: &str) -> Option<[f64; 3]> {
    parse_triplet_after(line, &["setpos_exact", "setpos"])
}

pub fn parse_angle_triplet(line: &str) -> Option<[f64; 3]> {
    parse_triplet_after(line, &["setang_exact", "setang"])
}

pub fn parse_position(line: &str) -> Option<PositionSnapshot> {
    let position = parse_position_triplet(line)?;
    let angles = parse_angle_triplet(line)?;

    Some(PositionSnapshot {
        x: position[0],
        y: position[1],
        z: position[2],
        pitch: angles[0],
        yaw: angles[1],
        roll: angles[2],
    })
}

#[derive(Default)]
pub struct PositionAssembler {
    position: Option<[f64; 3]>,
    angles: Option<[f64; 3]>,
}

impl PositionAssembler {
    pub fn push_line(&mut self, line: &str) -> Option<PositionSnapshot> {
        if let Some(position) = parse_position_triplet(line) {
            self.position = Some(position);
        }

        if let Some(angles) = parse_angle_triplet(line) {
            self.angles = Some(angles);
        }

        if self.position.is_none() || self.angles.is_none() {
            return None;
        }

        let position = self.position.take()?;
        let angles = self.angles.take()?;

        Some(PositionSnapshot {
            x: position[0],
            y: position[1],
            z: position[2],
            pitch: angles[0],
            yaw: angles[1],
            roll: angles[2],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_exact_commands_on_same_line() {
        let parsed = parse_position(
            "setpos_exact -123.5 456.25 78;setang_exact 12.5 -90 0",
        )
        .expect("position should parse");

        assert_eq!(
            parsed,
            PositionSnapshot {
                x: -123.5,
                y: 456.25,
                z: 78.0,
                pitch: 12.5,
                yaw: -90.0,
                roll: 0.0,
            }
        );
    }

    #[test]
    fn parses_legacy_commands_with_prefix_text() {
        let parsed = parse_position(
            "[Console] setpos 1 2 3; setang 4 5 6",
        )
        .expect("legacy position should parse");

        assert_eq!(
            parsed.to_deadlock_command(),
            "setpos_exact 1 2 3;setang_exact 4 5 6"
        );
    }

    #[test]
    fn assembles_position_from_two_lines() {
        let mut assembler = PositionAssembler::default();

        assert!(
            assembler
                .push_line("setpos_exact 10 20 30")
                .is_none()
        );

        let parsed = assembler
            .push_line("setang_exact 1 2 3")
            .expect("split position should assemble");

        assert_eq!(
            parsed,
            PositionSnapshot {
                x: 10.0,
                y: 20.0,
                z: 30.0,
                pitch: 1.0,
                yaw: 2.0,
                roll: 3.0,
            }
        );
    }

    #[test]
    fn ignores_unrelated_console_lines() {
        assert!(
            parse_position("bind scancode11 savestate_getpos")
                .is_none()
        );
    }
}