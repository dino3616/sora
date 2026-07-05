//! `.song`(ZIP + XML)のオフライン読解(§11.2.1「read」経路)。
//!
//! Studio One の `.song` は ZIP コンテナで、`Song/song.xml` に
//! テンポマップ・拍子・トラック・マーカーが入っている(実サンプルで確認済み)。
//! 開いているドキュメントには反映されない「最後に保存された状態」を返す。

use std::io::Read;
use std::path::Path;

use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};

use crate::error::DawError;
use crate::types::{DawMarker, DawProjectState, DawTrack};

/// `.song` を読み、プロジェクト状態を返す。
pub fn read_song(path: &Path) -> Result<DawProjectState, DawError> {
    let file = std::fs::File::open(path).map_err(|e| DawError::Io {
        path: path.to_path_buf(),
        source: e,
    })?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| DawError::SongParse {
        path: path.to_path_buf(),
        reason: format!("not a valid ZIP container: {e}"),
    })?;
    let mut xml = String::new();
    archive
        .by_name("Song/song.xml")
        .map_err(|e| DawError::SongParse {
            path: path.to_path_buf(),
            reason: format!("Song/song.xml not found: {e}"),
        })?
        .read_to_string(&mut xml)
        .map_err(|e| DawError::SongParse {
            path: path.to_path_buf(),
            reason: format!("Song/song.xml is not UTF-8: {e}"),
        })?;

    parse_song_xml(path, &xml)
}

fn attr(e: &BytesStart<'_>, name: &str) -> Option<String> {
    e.attributes().flatten().find_map(|a| {
        (a.key.as_ref() == name.as_bytes()).then(|| String::from_utf8_lossy(&a.value).to_string())
    })
}

fn parse_song_xml(path: &Path, xml: &str) -> Result<DawProjectState, DawError> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut bpm: Option<f64> = None;
    let mut time_signature: Option<String> = None;
    let mut sample_rate: Option<u32> = None;
    let mut tracks: Vec<DawTrack> = Vec::new();
    let mut markers: Vec<DawMarker> = Vec::new();
    // MediaTrack はネストしないが、クリップ(MusicPart 等)は内側に現れる
    let mut in_track = false;

    loop {
        let event = reader.read_event().map_err(|e| DawError::SongParse {
            path: path.to_path_buf(),
            reason: format!("XML parse error at byte {}: {e}", reader.buffer_position()),
        })?;
        match event {
            Event::Start(ref e) | Event::Empty(ref e) => {
                let is_empty = matches!(event, Event::Empty(_));
                match e.local_name().as_ref() {
                    // 最初のテンポセグメントを曲テンポとみなす
                    // (tempo 属性は 1 拍の秒数。60/tempo = BPM)
                    b"TempoMapSegment" if bpm.is_none() => {
                        if let Some(t) = attr(e, "tempo").and_then(|v| v.parse::<f64>().ok())
                            && t > 0.0
                        {
                            // 表示に耐える精度へ丸める(元値は倍精度の生値)
                            bpm = Some((60.0 / t * 1000.0).round() / 1000.0);
                        }
                    }
                    b"TimeSignatureMapSegment" if time_signature.is_none() => {
                        if let (Some(n), Some(d)) = (attr(e, "numerator"), attr(e, "denominator")) {
                            time_signature = Some(format!("{n}/{d}"));
                        }
                    }
                    b"Attributes" if sample_rate.is_none() => {
                        if attr(e, "sampleRate").is_some() {
                            sample_rate = attr(e, "sampleRate").and_then(|v| v.parse::<u32>().ok());
                        }
                    }
                    b"MediaTrack" => {
                        tracks.push(DawTrack {
                            id: attr(e, "trackID").unwrap_or_default(),
                            name: attr(e, "name").unwrap_or_default(),
                            kind: attr(e, "mediaType"),
                            color: attr(e, "color"),
                            clip_count: 0,
                        });
                        in_track = !is_empty;
                    }
                    b"MusicPart" | b"MusicPatternPart" | b"AudioPart" | b"AudioEvent"
                        if in_track =>
                    {
                        if let Some(track) = tracks.last_mut() {
                            track.clip_count += 1;
                        }
                    }
                    b"MarkerEvent" => {
                        markers.push(DawMarker {
                            name: attr(e, "name").unwrap_or_default(),
                            start_beats: attr(e, "start").and_then(|v| v.parse::<f64>().ok()),
                        });
                    }
                    _ => {}
                }
            }
            Event::End(ref e) => {
                if e.local_name().as_ref() == b"MediaTrack" {
                    in_track = false;
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }

    Ok(DawProjectState {
        source: path.to_path_buf(),
        bpm,
        time_signature,
        sample_rate,
        tracks,
        markers,
        notes: vec![],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<Song>
  <Attributes x:id="timeContext" sampleRate="44100" frameType="1">
    <TempoMap x:id="tempoMap">
      <TempoMapSegment curveType="0" start="0" end="1e+200" tempo="0.5"/>
    </TempoMap>
    <TimeSignatureMap x:id="timeSignatureMap">
      <TimeSignatureMapSegment start="0" numerator="7" denominator="8"/>
    </TimeSignatureMap>
  </Attributes>
  <List x:id="mediaTracks">
    <MediaTrack mediaType="Music" trackID="{AAA}" name="Bass" color="FFFC4700">
      <MusicPart clipID="{C1}" start="24" name="Bass"/>
      <MusicPart clipID="{C2}" start="28" name="Bass"/>
    </MediaTrack>
    <MediaTrack mediaType="Music" trackID="{BBB}" name="Kick" color="FFFFC693"/>
  </List>
  <MarkerTrack version="1" name="Marker">
    <MarkerEvent markerType="2" name="Start"/>
    <MarkerEvent markerType="3" start="598.5" name="End"/>
  </MarkerTrack>
</Song>"#;

    #[test]
    fn parses_tempo_tracks_and_markers() {
        let state = parse_song_xml(Path::new("test.song"), SAMPLE).unwrap();
        assert_eq!(state.bpm, Some(120.0));
        assert_eq!(state.time_signature.as_deref(), Some("7/8"));
        assert_eq!(state.sample_rate, Some(44100));
        assert_eq!(state.tracks.len(), 2);
        assert_eq!(state.tracks[0].name, "Bass");
        assert_eq!(state.tracks[0].clip_count, 2);
        assert_eq!(state.tracks[1].clip_count, 0);
        assert_eq!(state.markers.len(), 2);
        assert_eq!(state.markers[1].start_beats, Some(598.5));
    }

    #[test]
    fn rejects_non_zip_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bogus.song");
        std::fs::write(&path, b"not a zip").unwrap();
        let err = read_song(&path).unwrap_err();
        assert_eq!(err.code(), "SONG_PARSE_ERROR");
    }
}
