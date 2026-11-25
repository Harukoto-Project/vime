use crate::i18n::{tr, tr_args};
use std::{
    io,
    net::{Ipv4Addr, SocketAddrV4, UdpSocket},
};

pub const CHAT_LIMIT: usize = 144;

// VRChat's chatbox accepts an OSC string followed by the immediate-send and notification flags.
pub fn chat_packet(text: &str, notify: bool) -> io::Result<Vec<u8>> {
    if text.trim().is_empty() || text.contains('\0') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            tr("送信する文章を入力してください"),
        ));
    }
    if text.encode_utf16().count() > CHAT_LIMIT || text.lines().count() > 9 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            tr("VRChat の上限は144文字・9行です"),
        ));
    }
    let mut packet = Vec::new();
    osc_string(&mut packet, "/chatbox/input");
    osc_string(&mut packet, if notify { ",sTT" } else { ",sTF" });
    osc_string(&mut packet, text);
    Ok(packet)
}

pub fn send_osc(text: &str, port: u16, notify: bool) -> io::Result<()> {
    send_osc_packet(&chat_packet(text, notify)?, port)
}

pub fn send_osc_typing(port: u16, typing: bool) -> io::Result<()> {
    let mut packet = Vec::new();
    osc_string(&mut packet, "/chatbox/typing");
    osc_string(&mut packet, if typing { ",T" } else { ",F" });
    send_osc_packet(&packet, port)
}

fn send_osc_packet(packet: &[u8], port: u16) -> io::Result<()> {
    let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))?;
    socket.send_to(packet, SocketAddrV4::new(Ipv4Addr::LOCALHOST, port))?;
    Ok(())
}

fn osc_string(packet: &mut Vec<u8>, value: &str) {
    packet.extend_from_slice(value.as_bytes());
    packet.resize((packet.len() + 1).next_multiple_of(4), 0);
}

#[cfg(windows)]
pub fn send_scene_input(scene_pid: u32, text: &str) -> io::Result<()> {
    use windows_sys::Win32::UI::{
        Input::KeyboardAndMouse::{
            INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE,
            SendInput,
        },
        WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId},
    };

    if text.trim().is_empty() || text.contains('\0') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            tr("送信する文章を入力してください"),
        ));
    }
    let mut foreground_pid = 0;
    // Only insert while the current SteamVR scene owns the foreground window.
    unsafe { GetWindowThreadProcessId(GetForegroundWindow(), &mut foreground_pid) };
    if scene_pid == 0 || scene_pid != foreground_pid {
        return Err(io::Error::other(tr(
            "VRアプリの入力欄を開き、Windowsでそのアプリを前面にしてください",
        )));
    }
    let keys: Vec<_> = text
        .encode_utf16()
        .flat_map(|unit| {
            [KEYEVENTF_UNICODE, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP].map(|flags| INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wScan: unit,
                        dwFlags: flags,
                        ..Default::default()
                    },
                },
            })
        })
        .collect();
    let sent = unsafe { SendInput(keys.len() as u32, keys.as_ptr(), size_of::<INPUT>() as i32) };
    if sent != keys.len() as u32 {
        return Err(io::Error::other(tr_args(
            "Windows が入力イベントを受け取れませんでした ({0}/{1})",
            &[&sent.to_string(), &keys.len().to_string()],
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn sends_utf8_osc_to_a_loopback_receiver() {
        let receiver = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        receiver
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        send_osc("猫🌸", receiver.local_addr().unwrap().port(), false).unwrap();
        let mut buf = [0; 64];
        let count = receiver.recv(&mut buf).unwrap();
        assert_eq!(
            &buf[..count],
            b"/chatbox/input\0\0,sTF\0\0\0\0\xe7\x8c\xab\xf0\x9f\x8c\xb8\0"
        );
    }

    #[test]
    fn sends_typing_on_and_off_as_osc_booleans() {
        let receiver = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        receiver
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let port = receiver.local_addr().unwrap().port();
        let mut buf = [0; 64];
        for (typing, expected) in [
            (true, b"/chatbox/typing\0,T\0\0"),
            (false, b"/chatbox/typing\0,F\0\0"),
        ] {
            send_osc_typing(port, typing).unwrap();
            let count = receiver.recv(&mut buf).unwrap();
            assert_eq!(&buf[..count], expected);
        }
    }

    #[test]
    fn enforces_the_chatbox_budget_without_truncation() {
        assert!(chat_packet(&"😀".repeat(72), true).is_ok());
        assert!(chat_packet(&"😀".repeat(73), true).is_err());
        assert!(chat_packet(&"猫\n".repeat(10), true).is_err());
        assert!(chat_packet("  ", false).is_err());
    }
}
