//! EXIF ingestion: camera exposure parameters and capture timestamps.

use std::path::Path;

use exif::{In, Tag, Value};

/// Exposure-relevant EXIF data for one frame. Every field is optional;
/// missing data degrades gracefully upstream.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct FrameExif {
    /// Aperture as an f-number (e.g. 2.8).
    pub aperture: Option<f32>,
    /// Shutter speed in seconds.
    pub shutter_s: Option<f32>,
    /// ISO sensitivity.
    pub iso: Option<f32>,
    /// Capture time as unix epoch milliseconds (timezone-naive; only the
    /// relative spacing between frames matters).
    pub datetime_ms: Option<i64>,
}

/// Read the exposure EXIF of one image. Never fails: unreadable or absent
/// EXIF yields an empty `FrameExif`.
pub fn read_frame_exif(path: &Path) -> FrameExif {
    let generic = read_generic_exif(path);

    // Some RAW containers (e.g. ISO-BMFF-based ones) defeat the generic
    // reader; fall back to the RAW decoder's own metadata.
    #[cfg(feature = "raw")]
    if generic == FrameExif::default() && crate::source::is_raw_path(path) {
        if let Some(raw) = crate::raw::raw_exif(path) {
            return raw;
        }
    }

    generic
}

fn read_generic_exif(path: &Path) -> FrameExif {
    let Ok(file) = std::fs::File::open(path) else {
        return FrameExif::default();
    };
    let mut reader = std::io::BufReader::new(&file);
    let Ok(exif) = exif::Reader::new().read_from_container(&mut reader) else {
        return FrameExif::default();
    };

    // Cameras write Rational; other writers use Double/Float/Short. Accept
    // any numeric representation.
    let numeric = |tag: Tag| -> Option<f32> {
        exif.get_field(tag, In::PRIMARY)
            .and_then(|f| match &f.value {
                Value::Rational(v) if !v.is_empty() => Some(v[0].to_f32()),
                Value::SRational(v) if !v.is_empty() => Some(v[0].to_f32()),
                Value::Double(v) if !v.is_empty() => Some(v[0] as f32),
                Value::Float(v) if !v.is_empty() => Some(v[0]),
                other => other.get_uint(0).map(|u| u as f32),
            })
            .filter(|v| *v > 0.0)
    };

    let iso = numeric(Tag::PhotographicSensitivity);

    let datetime_ms = exif
        .get_field(Tag::DateTimeOriginal, In::PRIMARY)
        .or_else(|| exif.get_field(Tag::DateTime, In::PRIMARY))
        .and_then(|f| match &f.value {
            Value::Ascii(v) if !v.is_empty() => std::str::from_utf8(&v[0]).ok(),
            _ => None,
        })
        .and_then(parse_exif_datetime_ms)
        .map(|ms| {
            let subsec = exif
                .get_field(Tag::SubSecTimeOriginal, In::PRIMARY)
                .and_then(|f| match &f.value {
                    Value::Ascii(v) if !v.is_empty() => std::str::from_utf8(&v[0]).ok(),
                    _ => None,
                })
                .and_then(|s| {
                    let digits: String = s.trim().chars().take(3).collect();
                    let padded = format!("{digits:0<3}");
                    padded.parse::<i64>().ok()
                })
                .unwrap_or(0);
            ms + subsec
        });

    FrameExif {
        aperture: numeric(Tag::FNumber),
        shutter_s: numeric(Tag::ExposureTime),
        iso,
        datetime_ms,
    }
}

/// The camera exposure value at ISO sensitivity:
/// EV = log2(N² / t) − log2(ISO / 100). Higher EV means less light captured
/// (a darker capture of the same scene).
pub fn camera_ev(e: &FrameExif) -> Option<f32> {
    let (n, t, iso) = (e.aperture?, e.shutter_s?, e.iso?);
    Some((n * n / t).log2() - (iso / 100.0).log2())
}

/// Parse "YYYY:MM:DD HH:MM:SS" into unix epoch milliseconds (naive local
/// time — relative spacing is all the pipeline needs).
pub(crate) fn parse_exif_datetime_ms(s: &str) -> Option<i64> {
    let s = s.trim();
    let bytes: Vec<&str> = s.split([' ', ':']).collect();
    if bytes.len() != 6 {
        return None;
    }
    let year: i64 = bytes[0].parse().ok()?;
    let month: u32 = bytes[1].parse().ok()?;
    let day: u32 = bytes[2].parse().ok()?;
    let hour: i64 = bytes[3].parse().ok()?;
    let minute: i64 = bytes[4].parse().ok()?;
    let second: i64 = bytes[5].parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }

    let days = days_from_civil(year, month, day);
    Some((days * 86400 + hour * 3600 + minute * 60 + second) * 1000)
}

/// Days since 1970-01-01 (Howard Hinnant's algorithm).
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = if m > 2 { m - 3 } else { m + 9 } as i64;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    #[test]
    fn ev_formula_known_values() {
        // f/2.8, 1/100s, ISO 100: EV = log2(2.8^2 * 100) ~ 9.615
        let e = FrameExif {
            aperture: Some(2.8),
            shutter_s: Some(0.01),
            iso: Some(100.0),
            datetime_ms: None,
        };
        assert_relative_eq!(camera_ev(&e).unwrap(), 9.615, epsilon = 0.01);

        // Doubling ISO drops EV by exactly one stop.
        let e2 = FrameExif {
            iso: Some(200.0),
            ..e
        };
        assert_relative_eq!(
            camera_ev(&e).unwrap() - camera_ev(&e2).unwrap(),
            1.0,
            epsilon = 1e-5
        );

        // Doubling shutter time drops EV by exactly one stop.
        let e3 = FrameExif {
            shutter_s: Some(0.02),
            ..e
        };
        assert_relative_eq!(
            camera_ev(&e).unwrap() - camera_ev(&e3).unwrap(),
            1.0,
            epsilon = 1e-5
        );
    }

    #[test]
    fn missing_fields_yield_no_ev() {
        assert!(camera_ev(&FrameExif::default()).is_none());
    }

    #[test]
    fn datetime_parsing() {
        // 2024-01-01 00:00:00 UTC = 1704067200
        assert_eq!(
            parse_exif_datetime_ms("2024:01:01 00:00:00"),
            Some(1_704_067_200_000)
        );
        // One hour and one second later.
        assert_eq!(
            parse_exif_datetime_ms("2024:01:01 01:00:01"),
            Some(1_704_070_801_000)
        );
        assert!(parse_exif_datetime_ms("garbage").is_none());
        assert!(parse_exif_datetime_ms("2024:13:01 00:00:00").is_none());
    }

    #[test]
    fn unreadable_file_yields_default() {
        assert_eq!(
            read_frame_exif(Path::new("/nonexistent/file.jpg")),
            FrameExif::default()
        );
    }
}
