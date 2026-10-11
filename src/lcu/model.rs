use serde::Deserialize;
use serde_json::Value;
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Session {
    pub my_team: Vec<Player>,
    pub their_team: Vec<Player>,
    pub actions: Vec<Value>,
    pub local_player_cell_id: i64,
    pub timer: Timer,
    pub bans: Bans,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Player {
    pub cell_id: i64,
    pub champion_id: u32,
    pub champion_pick_intent: u32,
    pub assigned_position: String,
}
#[derive(Deserialize)]
pub(crate) struct Timer {
    pub phase: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Bans {
    pub my_team_bans: Vec<i64>,
    pub their_team_bans: Vec<i64>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Action {
    pub actor_cell_id: i64,
    pub champion_id: u32,
    pub completed: bool,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, alias = "inProgress")]
    pub is_in_progress: bool,
}
