use super::{
    decode::{decode_envelope, rpc_response},
    model::*,
};
use crate::{domain::Role, runtime::IntegrationError};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    io::Read,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub const ENDPOINT: &str = "https://mcp-api.op.gg/mcp";
fn public_connector() -> impl ureq::unversioned::transport::Connector<()> {
    // ureq's rustls adapter can repeat TCP reads within one complete_io call.
    // Put the absolute clock below TLS, not around the whole TLS operation.
    use ureq::unversioned::transport::{Connector, RustlsConnector, TcpConnector};
    ().chain(TcpConnector::default())
        .chain(AbsoluteDeadlineConnector)
        .chain(RustlsConnector::default())
}

#[derive(Debug)]
struct AbsoluteDeadlineConnector;
impl<T: ureq::unversioned::transport::Transport> ureq::unversioned::transport::Connector<T>
    for AbsoluteDeadlineConnector
{
    type Out = AbsoluteDeadlineTransport<T>;
    fn connect(
        &self,
        details: &ureq::unversioned::transport::ConnectionDetails,
        chained: Option<T>,
    ) -> Result<Option<Self::Out>, ureq::Error> {
        use ureq::unversioned::transport::time::Instant;
        let start = match details.now {
            Instant::Exact(start) => start,
            _ => std::time::Instant::now(),
        };
        let duration = (*details.timeout.after).min(Duration::from_secs(15));
        Ok(chained.map(|inner| AbsoluteDeadlineTransport {
            inner,
            deadline: start + duration,
        }))
    }
}
#[derive(Debug)]
struct AbsoluteDeadlineTransport<T> {
    inner: T,
    deadline: std::time::Instant,
}
impl<T> AbsoluteDeadlineTransport<T> {
    fn remaining(
        &self,
        timeout: ureq::unversioned::transport::NextTimeout,
    ) -> Result<ureq::unversioned::transport::NextTimeout, ureq::Error> {
        let remaining = self
            .deadline
            .saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            return Err(ureq::Error::Timeout(ureq::Timeout::Global));
        }
        Ok(ureq::unversioned::transport::NextTimeout {
            after: remaining.min(*timeout.after).into(),
            reason: ureq::Timeout::Global,
        })
    }
}
impl<T: ureq::unversioned::transport::Transport> ureq::unversioned::transport::Transport
    for AbsoluteDeadlineTransport<T>
{
    fn buffers(&mut self) -> &mut dyn ureq::unversioned::transport::Buffers {
        self.inner.buffers()
    }
    fn transmit_output(
        &mut self,
        amount: usize,
        timeout: ureq::unversioned::transport::NextTimeout,
    ) -> Result<(), ureq::Error> {
        let timeout = self.remaining(timeout)?;
        self.inner.transmit_output(amount, timeout)
    }
    fn await_input(
        &mut self,
        timeout: ureq::unversioned::transport::NextTimeout,
    ) -> Result<bool, ureq::Error> {
        let timeout = self.remaining(timeout)?;
        self.inner.await_input(timeout)
    }
    fn is_open(&mut self) -> bool {
        std::time::Instant::now() < self.deadline && self.inner.is_open()
    }
}

#[cfg(test)]
#[path = "opgg_deadline_tests.rs"]
mod deadline_tests;
fn invalid() -> IntegrationError {
    IntegrationError::Invalid("Estatísticas OP.GG incompletas ou incompatíveis")
}
pub fn now_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .min(i64::MAX as u64) as i64
}
pub struct HttpRequest {
    pub url: String,
    pub body: Option<Value>,
    pub headers: Vec<(String, String)>,
}
pub struct HttpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}
pub trait HttpTransport: Send {
    fn send(&self, request: HttpRequest) -> Result<HttpResponse, IntegrationError>;
}
pub struct PublicTransport {
    agent: ureq::Agent,
}
impl Default for PublicTransport {
    fn default() -> Self {
        let config = ureq::Agent::config_builder()
            .https_only(true)
            .max_redirects(0)
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(15)))
            // A connection-scoped deadline must not be reused by later calls.
            .max_idle_connections(0)
            .proxy(None)
            .build();
        Self {
            agent: ureq::Agent::with_parts(
                config,
                public_connector(),
                ureq::unversioned::resolver::DefaultResolver::default(),
            ),
        }
    }
}
impl HttpTransport for PublicTransport {
    fn send(&self, request: HttpRequest) -> Result<HttpResponse, IntegrationError> {
        if ![
            "https://mcp-api.op.gg/",
            "https://ddragon.leagueoflegends.com/",
        ]
        .iter()
        .any(|p| request.url.starts_with(p))
        {
            return Err(IntegrationError::Invalid("Fonte HTTPS não permitida"));
        }
        let mut response = if let Some(body) = request.body {
            let mut call = self
                .agent
                .post(&request.url)
                .header("Content-Type", "application/json");
            for (key, value) in &request.headers {
                call = call.header(key, value);
            }
            call.send(serde_json::to_vec(&body)?)
                .map_err(|_| IntegrationError::Network)?
        } else {
            let mut call = self.agent.get(&request.url);
            for (key, value) in &request.headers {
                call = call.header(key, value);
            }
            call.call().map_err(|_| IntegrationError::Network)?
        };
        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .filter_map(|(k, v)| v.to_str().ok().map(|v| (k.to_string(), v.to_owned())))
            .collect();
        let mut body = Vec::new();
        response
            .body_mut()
            .as_reader()
            .take(MAX_CACHE_BYTES + 1)
            .read_to_end(&mut body)
            .map_err(|_| IntegrationError::Network)?;
        if body.len() as u64 > MAX_CACHE_BYTES {
            return Err(IntegrationError::Invalid("Resposta excede 2 MiB"));
        }
        Ok(HttpResponse {
            status,
            headers,
            body,
        })
    }
}
pub fn checked_body(response: HttpResponse) -> Result<Vec<u8>, IntegrationError> {
    if !(200..300).contains(&response.status) {
        let retry_after = response
            .headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("retry-after"))
            .and_then(|(_, v)| retry_after_delay(v, SystemTime::now()));
        return Err(IntegrationError::Http {
            status: response.status,
            retry_after,
        });
    }
    if response.body.len() as u64 > MAX_CACHE_BYTES {
        return Err(IntegrationError::Invalid("Resposta excede 2 MiB"));
    }
    Ok(response.body)
}
pub fn retry_after_delay(value: &str, now: SystemTime) -> Option<Duration> {
    let value = value.trim();
    if let Ok(seconds) = value.parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }
    httpdate::parse_http_date(value)
        .ok()
        .map(|date| date.duration_since(now).unwrap_or_default())
}
fn get_json(transport: &dyn HttpTransport, url: String) -> Result<Value, IntegrationError> {
    Ok(serde_json::from_slice(&checked_body(transport.send(
        HttpRequest {
            url,
            body: None,
            headers: vec![],
        },
    )?)?)?)
}
pub fn parse_catalog(value: &Value) -> Result<Vec<ChampionIdentity>, IntegrationError> {
    let data = value
        .get("data")
        .and_then(Value::as_object)
        .ok_or_else(invalid)?;
    let mut output = Vec::new();
    let mut ids = BTreeSet::new();
    for row in data.values() {
        let id = row
            .get("key")
            .and_then(Value::as_str)
            .and_then(|v| v.parse::<u32>().ok())
            .filter(|v| *v > 0)
            .ok_or_else(invalid)?;
        let name = row
            .get("name")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(invalid)?;
        if !ids.insert(id) {
            return Err(invalid());
        }
        let api_name = name
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() {
                    c.to_ascii_uppercase()
                } else {
                    '_'
                }
            })
            .collect::<String>();
        let api_name = api_name
            .split('_')
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("_");
        output.push(ChampionIdentity {
            id,
            api_name,
            display_name: name.into(),
        });
    }
    if output.is_empty() {
        return Err(invalid());
    }
    Ok(output)
}
pub fn fetch_catalog(
    transport: &dyn HttpTransport,
    region: &str,
) -> Result<Vec<ChampionIdentity>, IntegrationError> {
    if ![
        "br", "na", "euw", "eune", "kr", "jp", "oce", "tr", "ru", "lan", "las",
    ]
    .contains(&region)
    {
        return Err(invalid());
    }
    let realm = get_json(
        transport,
        format!("https://ddragon.leagueoflegends.com/realms/{region}.json"),
    )?;
    let version = realm
        .pointer("/n/champion")
        .and_then(Value::as_str)
        .ok_or_else(invalid)?;
    if !version.bytes().all(|c| c.is_ascii_digit() || c == b'.') {
        return Err(invalid());
    }
    // English canonical names identify the OP.GG candidates consistently; UI may localize later.
    parse_catalog(&get_json(
        transport,
        format!("https://ddragon.leagueoflegends.com/cdn/{version}/data/en_US/champion.json"),
    )?)
}
fn canonical(s: &str) -> String {
    s.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}
pub fn role_api(role: Role) -> &'static str {
    match role {
        Role::Top => "top",
        Role::Jungle => "jungle",
        Role::Mid => "mid",
        Role::Bot => "adc",
        Role::Support => "support",
    }
}
pub fn parse_general(
    value: &Value,
    catalog: Vec<ChampionIdentity>,
    rank: Option<String>,
    now: i64,
) -> Result<MetaSnapshot, IntegrationError> {
    let positions = value
        .pointer("/data/positions")
        .and_then(Value::as_object)
        .ok_or_else(invalid)?;
    let mut stats = Vec::new();
    for role in Role::ALL {
        let rows = positions
            .get(role_api(role))
            .and_then(Value::as_array)
            .ok_or_else(invalid)?;
        for row in rows {
            let name = row
                .get("champion")
                .and_then(Value::as_str)
                .ok_or_else(invalid)?;
            let champ = catalog
                .iter()
                .find(|c| {
                    canonical(&c.display_name) == canonical(name)
                        || canonical(&c.api_name) == canonical(name)
                })
                .ok_or_else(invalid)?;
            let rate = row
                .get("win_rate")
                .and_then(Value::as_f64)
                .ok_or_else(invalid)? as f32;
            stats.push(RoleStats {
                champion_id: champ.id,
                role,
                games: row.get("play").and_then(Value::as_u64),
                wins: row.get("win").and_then(Value::as_u64),
                win_rate: rate,
                scope: SourceScope::default(),
            });
        }
    }
    let snapshot = MetaSnapshot {
        schema_version: 1,
        key: MetaKey {
            source: "OP.GG".into(),
            schema_version: 1,
            requested_rank: rank,
        },
        fetched_at: now,
        catalog,
        labels: StaticLabels::default(),
        stats,
        details: vec![],
    };
    snapshot.validate()?;
    Ok(snapshot)
}
fn ids(value: Option<&Value>) -> Result<Vec<u32>, IntegrationError> {
    match value {
        None | Some(Value::Null) => Ok(vec![]),
        Some(value) => value
            .as_array()
            .ok_or_else(invalid)?
            .iter()
            .map(|v| {
                v.as_u64()
                    .and_then(|n| u32::try_from(n).ok())
                    .filter(|n| *n > 0)
                    .ok_or_else(invalid)
            })
            .collect(),
    }
}
fn names(value: Option<&Value>) -> Result<Vec<String>, IntegrationError> {
    match value {
        None | Some(Value::Null) => Ok(vec![]),
        Some(v) => {
            let rows = v.as_array().ok_or_else(invalid)?;
            // Observed OP.GG output sometimes repeats numeric IDs in ids_names.
            // These are not names. Preserve IDs elsewhere and leave names unknown.
            if rows.iter().all(|v| v.as_u64().is_some()) {
                return Ok(vec![]);
            }
            rows.iter()
                .map(|v| v.as_str().map(str::to_owned).ok_or_else(invalid))
                .collect()
        }
    }
}
fn count_pair(v: &Value) -> (Option<u64>, Option<u64>) {
    (
        v.get("play").and_then(Value::as_u64),
        v.get("win").and_then(Value::as_u64),
    )
}
fn matchups(value: Option<&Value>) -> Result<Vec<Matchup>, IntegrationError> {
    let Some(value) = value.filter(|v| !v.is_null()) else {
        return Ok(vec![]);
    };
    value
        .as_array()
        .ok_or_else(invalid)?
        .iter()
        .map(|v| {
            let id = v
                .get("champion_id")
                .and_then(Value::as_u64)
                .and_then(|n| u32::try_from(n).ok())
                .ok_or_else(invalid)?;
            let (games, wins) = count_pair(v);
            Ok(Matchup {
                champion_id: id,
                role: None,
                games,
                wins,
                win_rate: v
                    .get("my_win_rate")
                    .or_else(|| v.get("win_rate"))
                    .and_then(Value::as_f64)
                    .map(|n| n as f32),
            })
        })
        .collect()
}
pub fn parse_details(
    value: &Value,
    champion: &ChampionIdentity,
    role: Role,
    rank: Option<&str>,
    now: i64,
) -> Result<ChampionDetails, IntegrationError> {
    let data = value
        .get("data")
        .and_then(Value::as_object)
        .ok_or_else(invalid)?;
    let mut runes = Vec::new();
    let mut spells = Vec::new();
    if let Some(row) = data.get("runes").filter(|v| !v.is_null()) {
        let mut rune_ids = Vec::new();
        let mut rune_names = Vec::new();
        for (ids_key, names_key) in [
            ("primary_rune_ids", "primary_rune_names"),
            ("secondary_rune_ids", "secondary_rune_names"),
            ("stat_mod_ids", "stat_mod_names"),
        ] {
            rune_ids.extend(ids(row.get(ids_key))?);
            rune_names.extend(names(row.get(names_key))?);
        }
        if !rune_ids.is_empty() {
            if rune_names.len() != rune_ids.len() {
                rune_names.clear();
            }
            let (games, wins) = count_pair(row);
            runes.push(BuildChoice {
                ids: rune_ids,
                names: rune_names,
                games,
                wins,
            });
        }
    }
    if let Some(row) = data.get("summoner_spells").filter(|v| !v.is_null()) {
        let spell_ids = ids(row.get("ids"))?;
        if !spell_ids.is_empty() {
            let (games, wins) = count_pair(row);
            spells.push(BuildChoice {
                ids: spell_ids,
                names: names(row.get("ids_names"))?,
                games,
                wins,
            });
        }
    }
    let scope = SourceScope {
        provider_timestamp: value
            .pointer("/data/trends/win/created_at")
            .and_then(Value::as_str)
            .map(str::to_owned),
        ..Default::default()
    };
    // A trend version is not sufficient proof for the patch/population of builds.
    let mut synergies = Vec::new();
    for partner_role in Role::ALL {
        if let Some(rows) = value
            .pointer(&format!("/data/synergies/{}", role_api(partner_role)))
            .and_then(Value::as_array)
        {
            for row in rows {
                let id = row
                    .get("synergy_champion_id")
                    .and_then(Value::as_u64)
                    .and_then(|v| u32::try_from(v).ok())
                    .ok_or_else(invalid)?;
                let (games, wins) = count_pair(row);
                synergies.push(Matchup {
                    champion_id: id,
                    role: Some(partner_role),
                    games,
                    wins,
                    win_rate: row
                        .get("win_rate")
                        .and_then(Value::as_f64)
                        .map(|v| v as f32),
                });
            }
        }
    }
    let details = ChampionDetails {
        champion_id: champion.id,
        role,
        requested_rank: rank.map(str::to_owned),
        scope,
        runes,
        spells,
        counters: matchups(data.get("weak_counters"))?,
        synergies,
        fetched_at: now,
    };
    details.validate()?;
    Ok(details)
}

pub trait MetaProvider: Send {
    fn fetch_general(&mut self) -> Result<MetaSnapshot, IntegrationError>;
    fn fetch_details(
        &mut self,
        champion: &ChampionIdentity,
        role: Role,
        rank: Option<&str>,
    ) -> Result<ChampionDetails, IntegrationError>;
}
pub struct OpggProvider<T: HttpTransport> {
    transport: T,
    catalog: Vec<ChampionIdentity>,
    rank: Option<String>,
    session: Option<String>,
    initialized: bool,
    next_id: u64,
    labels: StaticLabels,
}
impl<T: HttpTransport> OpggProvider<T> {
    pub fn new(transport: T, catalog: Vec<ChampionIdentity>, rank: Option<String>) -> Self {
        Self {
            transport,
            catalog,
            rank,
            session: None,
            initialized: false,
            next_id: 1,
            labels: StaticLabels::default(),
        }
    }
    pub fn with_labels(mut self, labels: StaticLabels) -> Self {
        self.labels = labels;
        self
    }
    fn request(
        &mut self,
        method: &str,
        params: Value,
        id: Option<u64>,
    ) -> Result<String, IntegrationError> {
        let mut body = json!({"jsonrpc":"2.0","method":method,"params":params});
        if let Some(id) = id {
            body["id"] = json!(id);
        }
        let mut headers = vec![(
            "Accept".into(),
            "application/json, text/event-stream".into(),
        )];
        if let Some(session) = &self.session {
            headers.push(("Mcp-Session-Id".into(), session.clone()));
        }
        let response = self.transport.send(HttpRequest {
            url: ENDPOINT.into(),
            body: Some(body),
            headers,
        })?;
        if let Some((_, v)) = response
            .headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("mcp-session-id"))
        {
            self.session = Some(v.clone());
        }
        String::from_utf8(checked_body(response)?).map_err(|_| invalid())
    }
    fn init(&mut self) -> Result<(), IntegrationError> {
        if self.initialized {
            return Ok(());
        }
        let id = self.next_id;
        self.next_id += 1;
        let response=self.request("initialize",json!({"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"Pikker","version":"0.1.0"}}),Some(id))?;
        rpc_response(&response, id)?;
        self.request("notifications/initialized", json!({}), None)?;
        let id = self.next_id;
        self.next_id += 1;
        let listing = self.request("tools/list", json!({}), Some(id))?;
        let listing = rpc_response(&listing, id)?;
        let tools = listing
            .get("tools")
            .and_then(Value::as_array)
            .ok_or_else(invalid)?;
        for required in ["lol_list_lane_meta_champions", "lol_get_champion_analysis"] {
            if !tools
                .iter()
                .any(|t| t.get("name").and_then(Value::as_str) == Some(required))
            {
                return Err(invalid());
            }
        }
        self.initialized = true;
        Ok(())
    }
    fn call(&mut self, name: &str, arguments: Value) -> Result<Value, IntegrationError> {
        self.init()?;
        let id = self.next_id;
        self.next_id += 1;
        decode_envelope(
            &self.request(
                "tools/call",
                json!({"name":name,"arguments":arguments}),
                Some(id),
            )?,
            id,
        )
    }
}
impl<T: HttpTransport> MetaProvider for OpggProvider<T> {
    fn fetch_general(&mut self) -> Result<MetaSnapshot, IntegrationError> {
        let fields: Vec<_> = Role::ALL
            .iter()
            .map(|r| {
                format!(
                    "data.positions.{}[].{{champion,play,win,win_rate}}",
                    role_api(*r)
                )
            })
            .collect();
        let value = self.call(
            "lol_list_lane_meta_champions",
            json!({"position":"all","desired_output_fields":fields}),
        )?;
        let mut snapshot = parse_general(
            &value,
            self.catalog.clone(),
            self.rank.clone(),
            now_seconds(),
        )?;
        snapshot.labels = self.labels.clone();
        Ok(snapshot)
    }
    fn fetch_details(
        &mut self,
        champion: &ChampionIdentity,
        role: Role,
        rank: Option<&str>,
    ) -> Result<ChampionDetails, IntegrationError> {
        let mut arguments = json!({"game_mode":"ranked","champion":champion.api_name,"position":role_api(role),
            "desired_output_fields":["champion","position","data.runes.{primary_rune_ids[],primary_rune_names[],secondary_rune_ids[],secondary_rune_names[],stat_mod_ids[],play,win}",
            "data.summoner_spells.{ids[],ids_names[],play,win}","data.weak_counters[].{champion_id,play,win,my_win_rate}",
            "data.trends.win.{version,created_at}"]});
        if let Some(rank) = rank {
            arguments["tier"] = json!(rank);
        }
        let fields = arguments["desired_output_fields"]
            .as_array_mut()
            .ok_or_else(invalid)?;
        for partner_role in Role::ALL {
            fields.push(json!(format!(
                "data.synergies.{}[].{{synergy_champion_id,play,win,win_rate}}",
                role_api(partner_role)
            )));
        }
        let value = self.call("lol_get_champion_analysis", arguments)?;
        let mut details = parse_details(&value, champion, role, rank, now_seconds())?;
        for build in &mut details.spells {
            if build.names.is_empty() {
                build.names = build
                    .ids
                    .iter()
                    .map(|id| {
                        self.labels
                            .spells
                            .get(id)
                            .cloned()
                            .unwrap_or_else(|| format!("Feitiço #{id}"))
                    })
                    .collect();
            }
        }
        for build in &mut details.runes {
            if build.names.is_empty() {
                build.names = build
                    .ids
                    .iter()
                    .map(|id| {
                        self.labels
                            .runes
                            .get(id)
                            .cloned()
                            .unwrap_or_else(|| format!("Fragmento/runa #{id}"))
                    })
                    .collect();
            }
        }
        Ok(details)
    }
}

pub fn parse_labels(
    spells: &Value,
    runes: &Value,
    version: &str,
) -> Result<StaticLabels, IntegrationError> {
    let mut labels = StaticLabels {
        catalog_version: Some(version.into()),
        ..Default::default()
    };
    for row in spells
        .get("data")
        .and_then(Value::as_object)
        .ok_or_else(invalid)?
        .values()
    {
        let id = row
            .get("key")
            .and_then(Value::as_str)
            .and_then(|s| s.parse::<u32>().ok())
            .filter(|n| *n > 0)
            .ok_or_else(invalid)?;
        let name = row
            .get("name")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(invalid)?;
        if labels.spells.insert(id, name.into()).is_some() {
            return Err(invalid());
        }
    }
    for style in runes.as_array().ok_or_else(invalid)? {
        for slot in style
            .get("slots")
            .and_then(Value::as_array)
            .ok_or_else(invalid)?
        {
            for rune in slot
                .get("runes")
                .and_then(Value::as_array)
                .ok_or_else(invalid)?
            {
                let id = rune
                    .get("id")
                    .and_then(Value::as_u64)
                    .and_then(|v| u32::try_from(v).ok())
                    .filter(|v| *v > 0)
                    .ok_or_else(invalid)?;
                let name = rune
                    .get("name")
                    .and_then(Value::as_str)
                    .filter(|s| !s.trim().is_empty())
                    .ok_or_else(invalid)?;
                if labels.runes.insert(id, name.into()).is_some() {
                    return Err(invalid());
                }
            }
        }
    }
    Ok(labels)
}
pub fn fetch_static_catalog(
    transport: &dyn HttpTransport,
    region: &str,
) -> Result<(Vec<ChampionIdentity>, StaticLabels), IntegrationError> {
    let catalog = fetch_catalog(transport, region)?;
    let realm = get_json(
        transport,
        format!("https://ddragon.leagueoflegends.com/realms/{region}.json"),
    )?;
    let version = realm
        .pointer("/n/champion")
        .and_then(Value::as_str)
        .ok_or_else(invalid)?;
    if !version.bytes().all(|c| c.is_ascii_digit() || c == b'.') {
        return Err(invalid());
    }
    let prefix = format!("https://ddragon.leagueoflegends.com/cdn/{version}/data/pt_BR");
    let spells = get_json(transport, format!("{prefix}/summoner.json"))?;
    let runes = get_json(transport, format!("{prefix}/runesReforged.json"))?;
    Ok((catalog, parse_labels(&spells, &runes, version)?))
}
