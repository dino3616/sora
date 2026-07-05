//! Sora Surface への仮想 MIDI トリガー送信(§11.2.1)。
//!
//! 実時間スケジューリング(sora-mcp::ops::send_file)とは別物で、
//! 数バイトのノートを即時送信するだけの軽量経路。送信後は必ず
//! Note Off を送る(サーフェスのトリガーはワンショットだが作法として)。

use std::time::Duration;

use anyhow::Context;
use midir::MidiOutput;

/// ノート On/Off の対を順に送る。ノート間は 150ms 空ける
/// (Studio One 側のコマンド実行が重ならないように)。
pub fn send_notes(port_name: &str, notes: &[u8]) -> anyhow::Result<()> {
    let output = MidiOutput::new("sora-trigger").context("initializing MIDI output")?;
    let ports = output.ports();
    let port = ports
        .iter()
        .find(|p| {
            output
                .port_name(p)
                .map(|n| n.contains(port_name))
                .unwrap_or(false)
        })
        .ok_or_else(|| {
            let available: Vec<String> = ports
                .iter()
                .filter_map(|p| output.port_name(p).ok())
                .collect();
            anyhow::anyhow!(
                "MIDI ポート `{port_name}` が見つかりません。利用可能: [{}]",
                available.join(", ")
            )
        })?;
    let mut conn = output
        .connect(port, "sora-trigger-out")
        .map_err(|e| anyhow::anyhow!("connecting to `{port_name}`: {e}"))?;

    for (i, note) in notes.iter().enumerate() {
        if i > 0 {
            std::thread::sleep(Duration::from_millis(150));
        }
        conn.send(&[0x90, *note, 0x7F])
            .map_err(|e| anyhow::anyhow!("sending note on: {e}"))?;
        std::thread::sleep(Duration::from_millis(30));
        conn.send(&[0x80, *note, 0x00])
            .map_err(|e| anyhow::anyhow!("sending note off: {e}"))?;
    }
    Ok(())
}
