use crate::system_one::Client;
use mdns_sd::{ServiceDaemon, ServiceEvent};
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    io,
    net::Ipv4Addr,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    thread,
    time::{Duration, Instant},
};

const SERVICE_TYPE: &str = "_oscjson._tcp.local.";
const REFRESH_INTERVAL: Duration = Duration::from_secs(5);

pub struct Discovery {
    stop: Arc<AtomicBool>,
    updates: Receiver<Option<u16>>,
    port: Option<u16>,
}

impl Discovery {
    pub fn start(wake: impl Fn() + Send + 'static) -> io::Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let cancelled = stop.clone();
        let (send, updates) = mpsc::channel();
        thread::Builder::new()
            .name("vime-oscquery".into())
            .spawn(move || {
                let Ok(daemon) = ServiceDaemon::new() else {
                    return;
                };
                if let Ok(events) = daemon.browse(SERVICE_TYPE) {
                    let mut candidates = BTreeMap::new();
                    let mut port = None;
                    let mut refresh_at = Instant::now();
                    while !cancelled.load(Ordering::Relaxed) && !events.is_disconnected() {
                        let changed = match events.recv_timeout(Duration::from_millis(250)) {
                            Ok(ServiceEvent::ServiceResolved(service)) => {
                                let name = service.get_fullname().to_ascii_lowercase();
                                let candidate = service
                                    .get_addresses_v4()
                                    .contains(&Ipv4Addr::LOCALHOST)
                                    .then_some(service.get_port())
                                    .filter(|port| *port != 0);
                                if name.starts_with("vrchat-client-") {
                                    match candidate {
                                        Some(tcp_port) => {
                                            candidates.insert(name, tcp_port) != Some(tcp_port)
                                        }
                                        None => candidates.remove(&name).is_some(),
                                    }
                                } else {
                                    false
                                }
                            }
                            Ok(ServiceEvent::ServiceRemoved(_, name)) => {
                                candidates.remove(&name.to_ascii_lowercase()).is_some()
                            }
                            _ => false,
                        };
                        if changed || Instant::now() >= refresh_at {
                            let next = candidates.iter().find_map(|(name, &tcp_port)| {
                                let instance = name.strip_suffix("._oscjson._tcp.local.")?;
                                resolve_port(instance, tcp_port)
                            });
                            refresh_at = Instant::now() + REFRESH_INTERVAL;
                            if next != port {
                                port = next;
                                if send.send(port).is_err() {
                                    break;
                                }
                                wake();
                            }
                        }
                    }
                }
                let _ = daemon.shutdown();
                if send.send(None).is_ok() {
                    wake();
                }
            })?;
        Ok(Self {
            stop,
            updates,
            port: None,
        })
    }

    pub fn poll(&mut self) -> bool {
        let Some(port) = self.updates.try_iter().last() else {
            return false;
        };
        let changed = self.port != port;
        self.port = port;
        changed
    }

    pub fn port(&self) -> Option<u16> {
        self.port
    }
}

impl Drop for Discovery {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
struct HostInfo {
    name: String,
    osc_ip: Option<Ipv4Addr>,
    osc_port: u16,
    osc_transport: Option<String>,
}

fn resolve_port(instance: &str, tcp_port: u16) -> Option<u16> {
    let response = Client::get_loopback_json(tcp_port, "/?HOST_INFO").ok()?;
    let info: HostInfo = serde_json::from_value(response).ok()?;
    // Windows VRChat serves OSCQuery on loopback. Match the name as another
    // machine can also announce a VRChat instance with a loopback address.
    (info.name.eq_ignore_ascii_case(instance)
        && info
            .osc_ip
            .is_none_or(|ip| ip == Ipv4Addr::LOCALHOST || ip.is_unspecified())
        && info
            .osc_transport
            .is_none_or(|transport| transport == "UDP")
        && info.osc_port != 0)
        .then_some(info.osc_port)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output;
    use std::{
        io::{BufRead, BufReader, Write},
        net::{TcpListener, UdpSocket},
    };

    #[test]
    fn sends_chatbox_to_the_udp_port_from_host_info() {
        let osc = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        osc.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let udp_port = osc.local_addr().unwrap().port();
        let http = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let tcp_port = http.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let (mut socket, _) = http.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = BufReader::new(&mut socket);
            let mut line = String::new();
            request.read_line(&mut line).unwrap();
            assert_eq!(line, "GET /?HOST_INFO HTTP/1.1\r\n");
            loop {
                line.clear();
                if request.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
                    break;
                }
            }
            let body = serde_json::json!({
                "NAME": "VRChat-Client-Test",
                "OSC_IP": "127.0.0.1",
                "OSC_PORT": udp_port,
                "OSC_TRANSPORT": "UDP",
            });
            let body = body.to_string();
            write!(
                socket,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len(),
            )
            .unwrap();
        });

        let port = resolve_port("vrchat-client-test", tcp_port);
        server.join().unwrap();
        assert_eq!(port, Some(udp_port));
        output::send_osc("猫", port.unwrap(), false).unwrap();
        let mut packet = [0; 64];
        let count = osc.recv(&mut packet).unwrap();
        assert_eq!(&packet[..count], output::chat_packet("猫", false).unwrap());
    }
}
