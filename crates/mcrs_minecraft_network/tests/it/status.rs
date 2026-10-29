use bevy_app::App;
use mcrs_minecraft_core::VERSION;
use mcrs_minecraft_network::packet_io::PacketIo;
use mcrs_minecraft_network::{BoundAddress, NetworkPlugin, StatusPlayers, StatusPublisher};
use mcrs_minecraft_protocol::handshake::Intent;
use mcrs_minecraft_protocol::packets::intent::serverbound::ServerboundHandshake;
use mcrs_minecraft_protocol::packets::status::clientbound::StatusResponse;
use mcrs_minecraft_protocol::packets::status::serverbound::StatusRequest;
use mcrs_minecraft_protocol::uuid::Uuid;
use mcrs_minecraft_protocol::{Bounded, VarInt};
use serde::Deserialize;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::time::Duration;

#[derive(Deserialize)]
struct Status {
    players: Players,
    description: Option<String>,
}

#[derive(Deserialize)]
struct Players {
    max: u32,
    online: u32,
    sample: Option<Vec<Sample>>,
}

#[derive(Deserialize, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Sample {
    id: Uuid,
    name: String,
}

fn server(max_players: u32, motd: &str, players: Vec<(Uuid, String)>) -> (App, SocketAddr) {
    let mut app = App::new();
    app.add_plugins(NetworkPlugin {
        address: SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0).into(),
        announce_on_lan: false,
        max_players,
        motd: motd.to_owned(),
        ..Default::default()
    });
    app.world()
        .resource::<StatusPublisher>()
        .publish(StatusPlayers { players });
    let address = app.world().resource::<BoundAddress>().0;
    app.update();
    (app, address)
}

fn status_json(address: SocketAddr) -> String {
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        tokio::time::timeout(Duration::from_secs(10), async {
            let mut io = PacketIo::connect(address).await.unwrap();
            io.send_packet(&ServerboundHandshake {
                protocol_version: VarInt(VERSION.protocol_version),
                server_address: Bounded("127.0.0.1"),
                server_port: address.port(),
                intent: Intent::Status,
            })
            .await
            .unwrap();
            io.send_packet(&StatusRequest).await.unwrap();
            io.recv_packet::<StatusResponse>()
                .await
                .unwrap()
                .json
                .to_owned()
        })
        .await
        .unwrap()
    })
}

#[test]
fn the_status_response_shows_the_settings_and_the_online_players() {
    let players = (1..=3)
        .map(|n| (Uuid::from_u128(n), format!("player{n}")))
        .collect();
    let (_app, address) = server(7, "§aTest", players);

    let json = status_json(address);
    let status: Status = serde_json::from_str(&json).unwrap();

    assert_eq!(status.players.max, 7);
    assert_eq!(status.players.online, 3);
    assert_eq!(status.description.as_deref(), Some("§aTest"));
    let mut sample = status.players.sample.unwrap();
    sample.sort();
    let expected: Vec<_> = (1..=3)
        .map(|n| Sample {
            id: Uuid::from_u128(n),
            name: format!("player{n}"),
        })
        .collect();
    assert_eq!(sample, expected);
    assert!(
        json.contains(r#"{"id":"00000000-0000-0000-0000-000000000001","name":"player1"}"#),
        "{json}"
    );
}

#[test]
fn the_sample_of_more_than_twelve_players_is_twelve_adjacent_ones() {
    let players = (0..15)
        .map(|n| (Uuid::from_u128(n), format!("player{n}")))
        .collect();
    let (_app, address) = server(20, "mcrs", players);

    let status: Status = serde_json::from_str(&status_json(address)).unwrap();

    assert_eq!(status.players.online, 15);
    let mut positions: Vec<u128> = status
        .players
        .sample
        .unwrap()
        .iter()
        .map(|sample| sample.id.as_u128())
        .collect();
    positions.sort_unstable();
    let start = *positions.first().unwrap();
    assert!(start <= 3, "{positions:?}");
    assert_eq!(positions, (start..start + 12).collect::<Vec<_>>());
}

#[test]
fn an_empty_server_with_no_motd_sends_neither_sample_nor_description() {
    let (_app, address) = server(20, "", Vec::new());

    let json = status_json(address);
    let status: Status = serde_json::from_str(&json).unwrap();

    assert_eq!(status.players.online, 0);
    assert!(status.players.sample.is_none(), "{json}");
    assert!(status.description.is_none(), "{json}");
}
