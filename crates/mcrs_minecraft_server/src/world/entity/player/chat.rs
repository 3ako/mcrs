use crate::world::entity::player::HostAnchor;
use bevy_app::{App, Plugin};
use bevy_ecs::message::MessageWriter;
use bevy_ecs::prelude::*;
use bevy_math::{DVec3, IVec3};
use mcrs_minecraft_core::{ResourceKey, ResourceLocation};
use mcrs_minecraft_engine::bus::to;
use mcrs_minecraft_engine::bus::{
    ArrivalCause, MovePayload, OutboundPlayerPacket, PacketPayload, PacketPriority, PacketTarget,
};
use mcrs_minecraft_engine::session::GameProfile;
use mcrs_minecraft_level::entity::InTransit;
use mcrs_minecraft_level::entity::physics::Transform;
use mcrs_minecraft_level::session::{Owner, PlayerSession};
use mcrs_minecraft_level::world::in_flight::MoveIds;
use mcrs_minecraft_network::event::Received;
use mcrs_minecraft_protocol::Text;
use mcrs_minecraft_protocol::packets::game::clientbound::ClientboundSystemChatPacket;
use mcrs_minecraft_protocol::packets::game::serverbound::{
    ServerboundChat, ServerboundChatCommand,
};
use mcrs_minecraft_protocol::setting::ChatMode;

use crate::client_info::ClientInfo;
use crate::world::entity::player::movement::{MAX_XZ, MAX_Y};
use mcrs_minecraft_dimension::Dimension;
use mcrs_minecraft_protocol::text::{Color, IntoText};
use mcrs_minecraft_worldgen_generator::stages::FillContext;
use tracing::info;

pub struct ChatPlugin;

impl Plugin for ChatPlugin {
    fn build(&self, app: &mut App) {
        app.add_observer(handle_chat);
        app.add_observer(handle_command);
    }
}

/// Primitive debug slash-command handler. The vanilla client sends
/// `ServerboundChatCommand` for any `/...` input (no command graph is
/// required for the client to transmit it). Commands run inside the
/// dimension sub-app, so any client-facing effect must route through the
/// bridge (`OutboundPlayerPacket`) rather than touching `ServerSideConnection`
/// directly, which is host-resident.
#[expect(
    clippy::too_many_lines,
    reason = "the debug slash-command handler matches each command in one place, where its effect and reply are read together"
)]
fn handle_command(
    event: On<Received<ServerboundChatCommand<'static>>>,
    mut sender_query: Query<(&HostAnchor, &mut Transform, &GameProfile, &Owner)>,
    mut packet_writer: MessageWriter<OutboundPlayerPacket>,
    move_sender: Res<
        mcrs_minecraft_level::world::channels::FromDimSender<
            mcrs_minecraft_engine::channel_types::FromDim,
        >,
    >,
    mut commands: Commands,
    fill: Option<Res<FillContext>>,
    mut move_ids: ResMut<MoveIds>,
) {
    let Some(pkt) = event.packet() else {
        return;
    };
    let command: &str = pkt.command.0;
    info!("command from {:?}: /{}", event.entity, command);
    let mut parts = command.split_whitespace();
    match parts.next() {
        Some("tp") => {
            let mut coords = parts.filter_map(|s| s.parse::<f64>().ok());
            let (Some(x), Some(y), Some(z), None) =
                (coords.next(), coords.next(), coords.next(), coords.next())
            else {
                return;
            };
            let pos = DVec3::new(x, y, z);
            let Ok((host_anchor, mut transform, _, _)) = sender_query.get_mut(event.entity) else {
                return;
            };
            let host = host_anchor.0;
            if !pos.is_finite() {
                packet_writer.write(to(
                    host,
                    PacketPayload::SystemChat(ClientboundSystemChatPacket {
                        content: Text::translate("parsing.double.expected", vec![])
                            .color(Color::RED),
                        overlay: false,
                    }),
                ));
                return;
            }

            if pos.x < -MAX_XZ
                || pos.x >= MAX_XZ
                || pos.y < -MAX_Y
                || pos.y >= MAX_Y
                || pos.z < -MAX_XZ
                || pos.z >= MAX_XZ
            {
                packet_writer.write(to(
                    host,
                    PacketPayload::SystemChat(ClientboundSystemChatPacket {
                        content: Text::translate("commands.teleport.invalidPosition", vec![])
                            .color(Color::RED),
                        overlay: false,
                    }),
                ));
                return;
            }

            transform.translation = pos;
            packet_writer.write(to(
                host,
                PacketPayload::SystemChat(ClientboundSystemChatPacket {
                    content: format!("Teleported to {:.1}, {:.1}, {:.1}", pos.x, pos.y, pos.z)
                        .into_text(),
                    overlay: false,
                }),
            ));
            info!("teleported {:?} to {:?}", event.entity, pos);
        }
        Some("dim") => {
            let Some(raw) = parts.next() else {
                return;
            };
            let target = match raw {
                "nether" | "the_nether" => {
                    mcrs_minecraft_dimension::keys::dimension::THE_NETHER.into()
                }
                "overworld" | "over" => mcrs_minecraft_dimension::keys::dimension::OVERWORLD.into(),
                "end" | "the_end" => mcrs_minecraft_dimension::keys::dimension::THE_END.into(),
                other => {
                    let Ok(location) = ResourceLocation::read(other) else {
                        return;
                    };
                    ResourceKey::<Dimension>::from_location(location)
                }
            };
            let Ok((_host_anchor, _transform, profile, owner)) = sender_query.get(event.entity)
            else {
                return;
            };
            let session = owner.0;
            // Source-allocated id: stamp the in-transit entity with the same id
            // the host echoes back on confirm/rollback so the source can match it.
            let move_id = move_ids.allocate();
            let payload = MovePayload::Player {
                uuid: profile.id,
                username: profile.username.clone(),
            };
            info!("dim move {:?} -> {}", event.entity, target);
            // Hide-on-move-out: keep the source entity live but excluded from every
            // source-dim system until the target confirms (despawn) or the move is
            // rolled back (un-hide). The entity is never despawned here.
            commands.entity(event.entity).insert(InTransit { move_id });
            let _ =
                move_sender
                    .0
                    .try_send(mcrs_minecraft_engine::channel_types::FromDim::MoveEntity {
                        move_id,
                        target,
                        cause: ArrivalCause::CommandTeleport {
                            pos: DVec3::new(0.0, 128.0, 0.0),
                        },
                        payload,
                        player: Some(session),
                    });
        }
        Some("locate") => {
            let (Some("structure"), Some(raw)) = (parts.next(), parts.next()) else {
                return;
            };
            let Ok((host_anchor, transform, _, _)) = sender_query.get(event.entity) else {
                return;
            };
            let id = if raw.contains(':') {
                raw.to_string()
            } else {
                format!("minecraft:{raw}")
            };
            let origin = transform.translation.floor().as_ivec3();
            packet_writer.write(to(
                host_anchor.0,
                PacketPayload::SystemChat(ClientboundSystemChatPacket {
                    content: locate_structure(fill.as_deref(), origin, &id),
                    overlay: false,
                }),
            ));
        }
        _ => {}
    }
}

fn locate_structure(fill: Option<&FillContext>, origin: IVec3, id: &str) -> Text {
    let not_found = || {
        Text::translate(
            "commands.locate.structure.not_found",
            vec![id.to_string().into_text()],
        )
        .color(Color::RED)
    };
    let Some(index) = fill.and_then(|fill| fill.structures.as_deref()) else {
        return not_found();
    };
    let Some(structure) = ResourceLocation::read(id)
        .ok()
        .and_then(|location| index.tables().frozen.structure_ids.get(&location).copied())
    else {
        return Text::translate(
            "commands.locate.structure.invalid",
            vec![id.to_string().into_text()],
        )
        .color(Color::RED);
    };
    let Some((found, _)) = index.locate(origin, &[structure]) else {
        return not_found();
    };
    let dx = found.x.wrapping_sub(origin.x);
    let dz = found.z.wrapping_sub(origin.z);
    let distance = (dx.wrapping_mul(dx).wrapping_add(dz.wrapping_mul(dz)) as f32)
        .sqrt()
        .floor() as i32;
    let coordinates = Text::translate(
        "chat.square_brackets",
        vec![Text::translate(
            "chat.coordinates",
            vec![
                found.x.to_string().into_text(),
                "~".into_text(),
                found.z.to_string().into_text(),
            ],
        )],
    )
    .color(Color::GREEN)
    .on_click_suggest_command(format!("/tp @s {} ~ {}", found.x, found.z))
    .on_hover_show_text(Text::translate("chat.coordinates.tooltip", vec![]));
    Text::translate(
        "commands.locate.structure.success",
        vec![
            id.to_string().into_text(),
            coordinates,
            distance.to_string().into_text(),
        ],
    )
}

fn handle_chat(
    event: On<Received<ServerboundChat<'static>>>,
    sender_query: Query<(&GameProfile, Option<&ClientInfo>, &HostAnchor)>,
    mut packet_writer: MessageWriter<OutboundPlayerPacket>,
) {
    let Some(pkt) = event.packet() else {
        return;
    };
    let Ok((profile, info, host_anchor)) = sender_query.get(event.entity) else {
        return;
    };
    let msg = pkt.message;
    if is_chat_message_illegal(&msg) {
        info!(
            player = %profile.username,
            "dropping chat message with illegal characters"
        );
        return;
    }

    if info.is_some_and(|info| info.chat_mode == ChatMode::Hidden) {
        packet_writer.write(to(
            host_anchor.0,
            PacketPayload::SystemChat(ClientboundSystemChatPacket {
                content: Text::translate("chat.disabled.options", vec![]).color(Color::RED),
                overlay: false,
            }),
        ));
        return;
    }

    let text = Text::translate(
        "chat.type.text",
        vec![
            profile.username.clone().into_text(),
            msg.to_string().into_text(),
        ],
    );
    info!("<{}> {}", profile.username, msg);

    packet_writer.write(OutboundPlayerPacket {
        target: PacketTarget::AllPlayers,
        priority: PacketPriority::Normal,
        data: PacketPayload::SystemChat(ClientboundSystemChatPacket {
            content: text,
            overlay: false,
        }),
        session: PlayerSession(0),
        epoch: 0,
    });
}

#[inline]
fn is_chat_message_illegal(msg: &str) -> bool {
    msg.chars().any(|c| !is_allowed_chat_character(c))
}

#[inline]
fn is_allowed_chat_character(ch: char) -> bool {
    !matches!(ch, '\0'..='\x1f' | '\x7f' | '§')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::entity::player::movement::{TeleportState, teleport};
    use bevy_ecs::message::Messages;
    use bevy_ecs::system::RunSystemOnce;
    use mcrs_minecraft_level::session::PlayerSession;
    use mcrs_minecraft_network::Instant;
    use mcrs_minecraft_protocol::Encode;
    use mcrs_minecraft_protocol::packets::game::clientbound::ClientboundPlayerPosition;

    fn world_with_player() -> (World, Entity) {
        let mut world = World::new();
        world.init_resource::<Messages<OutboundPlayerPacket>>();
        world.insert_resource(mcrs_minecraft_level::world::channels::FromDimSender(
            flume::unbounded::<mcrs_minecraft_engine::channel_types::FromDim>().0,
        ));
        world.insert_resource(MoveIds::new(Entity::PLACEHOLDER));
        world.add_observer(handle_command);
        let anchor = world.spawn_empty().id();
        let player = world
            .spawn((
                HostAnchor(anchor),
                Transform::default(),
                GameProfile {
                    id: Default::default(),
                    username: "tester".into(),
                    properties: vec![],
                },
                Owner(PlayerSession(1)),
                TeleportState::default(),
            ))
            .id();
        (world, player)
    }

    fn run_command(world: &mut World, player: Entity, command: &str) -> Vec<PacketPayload> {
        let mut body = Vec::new();
        ServerboundChatCommand {
            command: mcrs_minecraft_protocol::Bounded(command),
        }
        .encode(&mut body)
        .unwrap();
        world.trigger(Received::<ServerboundChatCommand<'static>>::new(
            player,
            body.into(),
            Instant::now(),
        ));
        world.flush();
        world.run_system_once(teleport).unwrap();
        world
            .resource_mut::<Messages<OutboundPlayerPacket>>()
            .drain()
            .map(|packet| packet.data)
            .collect()
    }

    fn positions(sent: &[PacketPayload]) -> Vec<&ClientboundPlayerPosition> {
        sent.iter()
            .filter_map(|payload| match payload {
                PacketPayload::PlayerPosition(position) => Some(position),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn tp_to_a_non_finite_or_out_of_world_position_leaves_the_player_in_place() {
        for command in [
            "tp NaN 0 0",
            "tp inf 0 0",
            "tp 0 -inf 0",
            "tp 30000000 0 0",
            "tp 0 20000000 0",
            "tp 0 0 -30000000.5",
        ] {
            let (mut world, player) = world_with_player();
            let sent = run_command(&mut world, player, command);
            assert!(positions(&sent).is_empty(), "{command}");
            assert_eq!(sent.len(), 1, "{command}: one error message");
            assert_eq!(
                world.get::<Transform>(player).unwrap().translation,
                DVec3::ZERO,
                "{command}"
            );
        }
    }

    #[test]
    fn tp_sends_one_position_with_the_next_teleport_id() {
        let (mut world, player) = world_with_player();
        let sent = run_command(&mut world, player, "tp 0 100 0");
        let positions = positions(&sent);
        assert_eq!(positions.len(), 1);
        assert_eq!(positions[0].teleport_id.0, 0);
        assert_eq!(positions[0].position, DVec3::new(0.0, 100.0, 0.0));
        let state = world.get::<TeleportState>(player).unwrap();
        assert_eq!(state.teleport_id_counter(), 1);
    }
}
