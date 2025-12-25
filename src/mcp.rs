use crate::gedcom::GedcomStore;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tracing::{info, warn};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    pub id: String,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Response {
    pub id: String,
    pub result: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub id: String,
    pub error: ErrorObject,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorObject {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OutboundMessage {
    Response(Response),
    Error(ErrorResponse),
}

#[derive(Debug, Default, Clone)]
pub struct Server {
    store: Option<Arc<Mutex<GedcomStore>>>,
    storage_path: Option<PathBuf>,
}

impl Server {
    pub fn new(store: Option<GedcomStore>) -> Self {
        Self {
            store: store.map(|s| Arc::new(Mutex::new(s))),
            storage_path: None,
        }
    }

    pub fn with_storage(store: GedcomStore, storage_path: PathBuf) -> Self {
        Self {
            store: Some(Arc::new(Mutex::new(store))),
            storage_path: Some(storage_path),
        }
    }

    pub fn handle_request(&self, request: Request) -> OutboundMessage {
        info!(
            "handling request id={} method={}",
            request.id, request.method
        );
        match request.method.as_str() {
            "ping" => OutboundMessage::Response(Response {
                id: request.id,
                result: serde_json::json!({ "status": "ok" }),
            }),
            "get_individual" => self.handle_get_individual(request),
            "get_family" => self.handle_get_family(request),
            "list_individuals" => self.handle_list_individuals(request),
            "list_families" => self.handle_list_families(request),
            "create_individual" => self.handle_create_individual(request),
            "create_family" => self.handle_create_family(request),
            other => {
                warn!("method not found: {}", other);
                OutboundMessage::Error(ErrorResponse::method_not_found(request.id, other))
            }
        }
    }

    pub fn handle_raw_message(&self, input: &str) -> OutboundMessage {
        match parse_request(input) {
            Ok(request) => self.handle_request(request),
            Err(err) => {
                warn!("failed to parse request: {err}");
                OutboundMessage::Error(ErrorResponse::parse_error(err.to_string()))
            }
        }
    }

    pub fn handle_json_line(&self, input: &str) -> Result<String, serde_json::Error> {
        let message = self.handle_raw_message(input);
        serialize_message(&message)
    }

    pub fn serve_lines<R: BufRead, W: Write>(
        &self,
        reader: R,
        mut writer: W,
    ) -> Result<(), std::io::Error> {
        for line in reader.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }

            let output = match self.handle_json_line(&line) {
                Ok(out) => out,
                Err(err) => serialize_message(&OutboundMessage::Error(ErrorResponse::parse_error(
                    err.to_string(),
                )))
                .unwrap_or_else(|_| {
                    serde_json::json!({
                        "type": "error",
                        "id": "null",
                        "error": { "code": -32700, "message": err.to_string() }
                    })
                    .to_string()
                }),
            };

            writeln!(writer, "{output}")?;
        }

        Ok(())
    }

    fn handle_get_individual(&self, request: Request) -> OutboundMessage {
        let id = request
            .params
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_owned);

        let Some(id) = id else {
            return OutboundMessage::Error(ErrorResponse::invalid_params(
                request.id,
                "missing required param: id",
            ));
        };

        let Some(store) = &self.store else {
            return OutboundMessage::Error(ErrorResponse::server_error(
                request.id,
                "server not initialized with GEDCOM data",
            ));
        };

        let guard = match store.lock() {
            Ok(guard) => guard,
            Err(_) => {
                return OutboundMessage::Error(ErrorResponse::server_error(
                    request.id,
                    "store lock poisoned",
                ));
            }
        };

        match guard.get_individual(&id) {
            Some(individual) => OutboundMessage::Response(Response {
                id: request.id,
                result: serde_json::to_value(individual).unwrap_or_else(|_| {
                    serde_json::json!({
                        "id": individual.id,
                        "name": individual.name,
                        "birth": individual.birth,
                        "death": individual.death
                    })
                }),
            }),
            None => OutboundMessage::Error(ErrorResponse::not_found(
                request.id,
                format!("individual {id} not found"),
            )),
        }
    }

    fn handle_create_individual(&self, request: Request) -> OutboundMessage {
        let Some(store) = &self.store else {
            return OutboundMessage::Error(ErrorResponse::server_error(
                request.id,
                "server not initialized with GEDCOM data",
            ));
        };

        let mut guard = match store.lock() {
            Ok(guard) => guard,
            Err(_) => {
                return OutboundMessage::Error(ErrorResponse::server_error(
                    request.id,
                    "store lock poisoned",
                ));
            }
        };

        let Some(id) = request.params.get("id").and_then(Value::as_str) else {
            return OutboundMessage::Error(ErrorResponse::invalid_params(
                request.id,
                "missing required param: id",
            ));
        };

        let name = request
            .params
            .get("name")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let birth = parse_event(request.params.get("birth"));
        let death = parse_event(request.params.get("death"));

        let individual = crate::gedcom::Individual {
            id: id.to_owned(),
            name,
            birth,
            death,
        };

        match guard.insert_individual(individual.clone()) {
            Ok(_) => {
                let snapshot = guard.to_data();
                drop(guard);
                if let Some(path) = &self.storage_path {
                    if let Err(err) = persist_snapshot(path, &snapshot) {
                        return OutboundMessage::Error(ErrorResponse::server_error(
                            request.id,
                            format!("failed to persist data: {err}"),
                        ));
                    }
                }

                OutboundMessage::Response(Response {
                    id: request.id,
                    result: serde_json::to_value(individual).unwrap_or_else(|_| Value::Null),
                })
            }
            Err(crate::gedcom::StoreError::DuplicateIndividual(existing)) => {
                OutboundMessage::Error(ErrorResponse::conflict(
                    request.id,
                    format!("individual {existing} already exists"),
                ))
            }
            Err(_) => OutboundMessage::Error(ErrorResponse::server_error(
                request.id,
                "failed to insert individual",
            )),
        }
    }

    fn handle_list_individuals(&self, request: Request) -> OutboundMessage {
        let Some(store) = &self.store else {
            return OutboundMessage::Error(ErrorResponse::server_error(
                request.id,
                "server not initialized with GEDCOM data",
            ));
        };

        let guard = match store.lock() {
            Ok(guard) => guard,
            Err(_) => {
                return OutboundMessage::Error(ErrorResponse::server_error(
                    request.id,
                    "store lock poisoned",
                ));
            }
        };

        let items: Vec<_> = guard.individuals().cloned().collect();

        OutboundMessage::Response(Response {
            id: request.id,
            result: serde_json::to_value(items).unwrap_or_else(|_| Value::Null),
        })
    }

    fn handle_list_families(&self, request: Request) -> OutboundMessage {
        let Some(store) = &self.store else {
            return OutboundMessage::Error(ErrorResponse::server_error(
                request.id,
                "server not initialized with GEDCOM data",
            ));
        };

        let guard = match store.lock() {
            Ok(guard) => guard,
            Err(_) => {
                return OutboundMessage::Error(ErrorResponse::server_error(
                    request.id,
                    "store lock poisoned",
                ));
            }
        };

        let items: Vec<_> = guard.families().cloned().collect();

        OutboundMessage::Response(Response {
            id: request.id,
            result: serde_json::to_value(items).unwrap_or_else(|_| Value::Null),
        })
    }

    fn handle_get_family(&self, request: Request) -> OutboundMessage {
        let id = request
            .params
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_owned);

        let Some(id) = id else {
            return OutboundMessage::Error(ErrorResponse::invalid_params(
                request.id,
                "missing required param: id",
            ));
        };

        let Some(store) = &self.store else {
            return OutboundMessage::Error(ErrorResponse::server_error(
                request.id,
                "server not initialized with GEDCOM data",
            ));
        };

        let guard = match store.lock() {
            Ok(guard) => guard,
            Err(_) => {
                return OutboundMessage::Error(ErrorResponse::server_error(
                    request.id,
                    "store lock poisoned",
                ));
            }
        };

        match guard.get_family(&id) {
            Some(family) => OutboundMessage::Response(Response {
                id: request.id,
                result: serde_json::to_value(family).unwrap_or_else(|_| {
                    serde_json::json!({
                        "id": family.id,
                        "husband": family.husband,
                        "wife": family.wife,
                        "children": family.children
                    })
                }),
            }),
            None => OutboundMessage::Error(ErrorResponse::not_found(
                request.id,
                format!("family {id} not found"),
            )),
        }
    }

    fn handle_create_family(&self, request: Request) -> OutboundMessage {
        let Some(store) = &self.store else {
            return OutboundMessage::Error(ErrorResponse::server_error(
                request.id,
                "server not initialized with GEDCOM data",
            ));
        };

        let mut guard = match store.lock() {
            Ok(guard) => guard,
            Err(_) => {
                return OutboundMessage::Error(ErrorResponse::server_error(
                    request.id,
                    "store lock poisoned",
                ));
            }
        };

        let Some(id) = request.params.get("id").and_then(Value::as_str) else {
            return OutboundMessage::Error(ErrorResponse::invalid_params(
                request.id,
                "missing required param: id",
            ));
        };

        let husband = request
            .params
            .get("husband")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let wife = request
            .params
            .get("wife")
            .and_then(Value::as_str)
            .map(str::to_owned);

        let children = match request.params.get("children") {
            Some(Value::Array(arr)) => {
                let mut children = Vec::new();
                for child in arr {
                    if let Some(cid) = child.as_str() {
                        children.push(cid.to_owned());
                    } else {
                        return OutboundMessage::Error(ErrorResponse::invalid_params(
                            request.id,
                            "children must be an array of strings",
                        ));
                    }
                }
                children
            }
            Some(_) => {
                return OutboundMessage::Error(ErrorResponse::invalid_params(
                    request.id,
                    "children must be an array of strings",
                ));
            }
            None => Vec::new(),
        };

        let family = crate::gedcom::Family {
            id: id.to_owned(),
            husband,
            wife,
            children,
        };

        match guard.insert_family(family.clone()) {
            Ok(_) => {
                let snapshot = guard.to_data();
                drop(guard);
                if let Some(path) = &self.storage_path {
                    if let Err(err) = persist_snapshot(path, &snapshot) {
                        return OutboundMessage::Error(ErrorResponse::server_error(
                            request.id,
                            format!("failed to persist data: {err}"),
                        ));
                    }
                }

                OutboundMessage::Response(Response {
                    id: request.id,
                    result: serde_json::to_value(family).unwrap_or_else(|_| Value::Null),
                })
            }
            Err(crate::gedcom::StoreError::DuplicateFamily(existing)) => OutboundMessage::Error(
                ErrorResponse::conflict(request.id, format!("family {existing} already exists")),
            ),
            Err(_) => OutboundMessage::Error(ErrorResponse::server_error(
                request.id,
                "failed to insert family",
            )),
        }
    }
}

fn parse_event(value: Option<&Value>) -> Option<crate::gedcom::Event> {
    let Value::Object(map) = value? else {
        return None;
    };

    let date = map.get("date").and_then(Value::as_str).map(str::to_owned);
    let place = map.get("place").and_then(Value::as_str).map(str::to_owned);

    if date.is_none() && place.is_none() {
        None
    } else {
        Some(crate::gedcom::Event { date, place })
    }
}

fn persist_snapshot(
    path: &PathBuf,
    data: &crate::gedcom::GedcomData,
) -> Result<(), std::io::Error> {
    let tmp_path = path.with_extension("tmp");
    {
        let mut file = fs::File::create(&tmp_path)?;
        serde_json::to_writer_pretty(&mut file, data)
            .map_err(|err| std::io::Error::new(std::io::ErrorKind::Other, err))?;
        file.sync_all()?;
    }
    fs::rename(tmp_path, path)?;
    Ok(())
}

impl ErrorResponse {
    pub fn method_not_found(id: String, method: impl Into<String>) -> Self {
        Self {
            id,
            error: ErrorObject {
                code: -32601, // JSON-RPC method not found
                message: format!("method not found: {}", method.into()),
                data: None,
            },
        }
    }

    pub fn parse_error(message: impl Into<String>) -> Self {
        Self {
            id: "null".into(),
            error: ErrorObject {
                code: -32700, // JSON-RPC parse error
                message: message.into(),
                data: None,
            },
        }
    }

    pub fn invalid_params(id: String, message: impl Into<String>) -> Self {
        Self {
            id,
            error: ErrorObject {
                code: -32602,
                message: message.into(),
                data: None,
            },
        }
    }

    pub fn server_error(id: String, message: impl Into<String>) -> Self {
        Self {
            id,
            error: ErrorObject {
                code: -32000,
                message: message.into(),
                data: None,
            },
        }
    }

    pub fn not_found(id: String, message: impl Into<String>) -> Self {
        Self {
            id,
            error: ErrorObject {
                code: -32004,
                message: message.into(),
                data: None,
            },
        }
    }

    pub fn conflict(id: String, message: impl Into<String>) -> Self {
        Self {
            id,
            error: ErrorObject {
                code: -32001,
                message: message.into(),
                data: None,
            },
        }
    }
}

pub fn parse_request(input: &str) -> Result<Request, serde_json::Error> {
    serde_json::from_str(input)
}

pub fn serialize_message(message: &OutboundMessage) -> Result<String, serde_json::Error> {
    serde_json::to_string(message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gedcom::{Family, GedcomData, GedcomStore, Individual};
    use tempfile;

    #[test]
    fn round_trips_request_json() {
        let json = r#"{"id":"1","method":"ping","params":{"echo":"hi"}}"#;
        let request = parse_request(json).expect("should parse");
        assert_eq!(
            request,
            Request {
                id: "1".into(),
                method: "ping".into(),
                params: serde_json::json!({"echo": "hi"})
            }
        );
        let serialized = serde_json::to_string(&request).expect("should serialize");
        assert_eq!(
            serde_json::from_str::<Request>(&serialized).unwrap(),
            request
        );
    }

    #[test]
    fn handles_ping_request() {
        let server = Server::default();
        let response = server.handle_request(Request {
            id: "1".into(),
            method: "ping".into(),
            params: Value::Null,
        });

        assert_eq!(
            response,
            OutboundMessage::Response(Response {
                id: "1".into(),
                result: serde_json::json!({ "status": "ok" })
            })
        );
    }

    #[test]
    fn returns_error_for_unknown_method() {
        let server = Server::default();
        let response = server.handle_request(Request {
            id: "2".into(),
            method: "unknown".into(),
            params: Value::Null,
        });

        match response {
            OutboundMessage::Error(error) => {
                assert_eq!(error.id, "2");
                assert_eq!(error.error.code, -32601);
                assert!(error.error.message.contains("method not found"));
            }
            other => panic!("expected error response, got {other:?}"),
        }
    }

    #[test]
    fn serializes_outbound_message() {
        let message = OutboundMessage::Response(Response {
            id: "3".into(),
            result: serde_json::json!({"status": "ok"}),
        });

        let json = serialize_message(&message).expect("should serialize");
        assert!(json.contains("\"status\":\"ok\""));
        let deserialized: OutboundMessage =
            serde_json::from_str(&json).expect("should deserialize");
        assert_eq!(deserialized, message);
    }

    #[test]
    fn returns_parse_error_for_invalid_json() {
        let server = Server::default();
        let response = server.handle_raw_message("{ invalid json");

        match response {
            OutboundMessage::Error(error) => {
                assert_eq!(error.error.code, -32700);
                assert!(
                    !error.error.message.is_empty(),
                    "parse error message should be present"
                );
            }
            other => panic!("expected parse error, got {other:?}"),
        }
    }

    #[test]
    fn processes_json_line_happy_path() {
        let server = Server::default();
        let raw = r#"{"id":"1","method":"ping","params":{}}"#;
        let output = server.handle_json_line(raw).expect("should serialize");
        let message: OutboundMessage =
            serde_json::from_str(&output).expect("should deserialize outbound");

        match message {
            OutboundMessage::Response(resp) => {
                assert_eq!(resp.id, "1");
                assert_eq!(resp.result, serde_json::json!({"status": "ok"}));
            }
            other => panic!("expected response, got {other:?}"),
        }
    }

    #[test]
    fn processes_json_line_with_parse_error() {
        let server = Server::default();
        let output = server
            .handle_json_line("{ invalid json")
            .expect("serialize error response");

        let message: OutboundMessage =
            serde_json::from_str(&output).expect("should deserialize error");

        match message {
            OutboundMessage::Error(err) => {
                assert_eq!(err.error.code, -32700);
                assert!(
                    !err.error.message.is_empty(),
                    "error message should be present"
                );
            }
            other => panic!("expected error, got {other:?}"),
        }
    }

    fn build_store() -> GedcomStore {
        let data = GedcomData {
            individuals: vec![Individual {
                id: "I1".into(),
                name: Some("Indexed".into()),
                birth: Some(crate::gedcom::Event {
                    date: Some("1 JAN 1900".into()),
                    place: None,
                }),
                death: None,
            }],
            families: vec![],
        };
        GedcomStore::from_data(data)
    }

    #[test]
    fn returns_individual_details() {
        let server = Server::new(Some(build_store()));
        let response = server.handle_request(Request {
            id: "42".into(),
            method: "get_individual".into(),
            params: serde_json::json!({"id": "I1"}),
        });

        match response {
            OutboundMessage::Response(resp) => {
                assert_eq!(resp.id, "42");
                assert_eq!(
                    resp.result,
                    serde_json::json!({
                        "id": "I1",
                        "name": "Indexed",
                        "birth": {
                            "date": "1 JAN 1900",
                            "place": null
                        },
                        "death": null
                    })
                );
            }
            other => panic!("expected response, got {other:?}"),
        }
    }

    #[test]
    fn errors_when_id_missing() {
        let server = Server::new(Some(build_store()));
        let response = server.handle_request(Request {
            id: "43".into(),
            method: "get_individual".into(),
            params: serde_json::json!({}),
        });

        match response {
            OutboundMessage::Error(err) => {
                assert_eq!(err.id, "43");
                assert_eq!(err.error.code, -32602);
            }
            other => panic!("expected error, got {other:?}"),
        }
    }

    #[test]
    fn errors_when_individual_not_found() {
        let server = Server::new(Some(build_store()));
        let response = server.handle_request(Request {
            id: "44".into(),
            method: "get_individual".into(),
            params: serde_json::json!({"id": "missing"}),
        });

        match response {
            OutboundMessage::Error(err) => {
                assert_eq!(err.id, "44");
                assert_eq!(err.error.code, -32004);
            }
            other => panic!("expected error, got {other:?}"),
        }
    }

    #[test]
    fn errors_when_store_missing() {
        let server = Server::default();
        let response = server.handle_request(Request {
            id: "45".into(),
            method: "get_individual".into(),
            params: serde_json::json!({"id": "I1"}),
        });

        match response {
            OutboundMessage::Error(err) => {
                assert_eq!(err.id, "45");
                assert_eq!(err.error.code, -32000);
            }
            other => panic!("expected error, got {other:?}"),
        }
    }

    fn build_family_store() -> GedcomStore {
        let data = GedcomData {
            individuals: vec![],
            families: vec![Family {
                id: "F1".into(),
                husband: Some("I1".into()),
                wife: Some("I2".into()),
                children: vec!["I3".into()],
            }],
        };
        GedcomStore::from_data(data)
    }

    fn empty_store() -> GedcomStore {
        GedcomStore::from_data(GedcomData {
            individuals: vec![],
            families: vec![],
        })
    }

    #[test]
    fn lists_individuals() {
        let server = Server::new(Some(build_store()));
        let response = server.handle_request(Request {
            id: "200".into(),
            method: "list_individuals".into(),
            params: Value::Null,
        });

        match response {
            OutboundMessage::Response(resp) => {
                assert_eq!(resp.id, "200");
                assert!(resp.result.is_array());
                let arr = resp.result.as_array().unwrap();
                assert_eq!(arr.len(), 1);
                assert_eq!(arr[0]["id"], "I1");
            }
            other => panic!("expected response, got {other:?}"),
        }
    }

    #[test]
    fn lists_families() {
        let server = Server::new(Some(build_family_store()));
        let response = server.handle_request(Request {
            id: "201".into(),
            method: "list_families".into(),
            params: Value::Null,
        });

        match response {
            OutboundMessage::Response(resp) => {
                assert_eq!(resp.id, "201");
                assert!(resp.result.is_array());
                let arr = resp.result.as_array().unwrap();
                assert_eq!(arr.len(), 1);
                assert_eq!(arr[0]["id"], "F1");
            }
            other => panic!("expected response, got {other:?}"),
        }
    }

    #[test]
    fn returns_family_details() {
        let server = Server::new(Some(build_family_store()));
        let response = server.handle_request(Request {
            id: "100".into(),
            method: "get_family".into(),
            params: serde_json::json!({"id": "F1"}),
        });

        match response {
            OutboundMessage::Response(resp) => {
                assert_eq!(resp.id, "100");
                assert_eq!(
                    resp.result,
                    serde_json::json!({
                        "id": "F1",
                        "husband": "I1",
                        "wife": "I2",
                        "children": ["I3"]
                    })
                );
            }
            other => panic!("expected response, got {other:?}"),
        }
    }

    #[test]
    fn errors_when_family_missing() {
        let server = Server::new(Some(build_family_store()));
        let response = server.handle_request(Request {
            id: "101".into(),
            method: "get_family".into(),
            params: serde_json::json!({"id": "missing"}),
        });

        match response {
            OutboundMessage::Error(err) => {
                assert_eq!(err.id, "101");
                assert_eq!(err.error.code, -32004);
            }
            other => panic!("expected error, got {other:?}"),
        }
    }

    #[test]
    fn errors_when_family_param_missing() {
        let server = Server::new(Some(build_family_store()));
        let response = server.handle_request(Request {
            id: "102".into(),
            method: "get_family".into(),
            params: serde_json::json!({}),
        });

        match response {
            OutboundMessage::Error(err) => {
                assert_eq!(err.id, "102");
                assert_eq!(err.error.code, -32602);
            }
            other => panic!("expected error, got {other:?}"),
        }
    }

    #[test]
    fn errors_when_store_missing_for_family() {
        let server = Server::default();
        let response = server.handle_request(Request {
            id: "103".into(),
            method: "get_family".into(),
            params: serde_json::json!({"id": "F1"}),
        });

        match response {
            OutboundMessage::Error(err) => {
                assert_eq!(err.id, "103");
                assert_eq!(err.error.code, -32000);
            }
            other => panic!("expected error, got {other:?}"),
        }
    }

    #[test]
    fn creates_individual() {
        let server = Server::new(Some(empty_store()));
        let response = server.handle_request(Request {
            id: "300".into(),
            method: "create_individual".into(),
            params: serde_json::json!({
                "id": "I99",
                "name": "New Person",
                "birth": { "date": "1 JAN 1990", "place": "Town" }
            }),
        });

        match response {
            OutboundMessage::Response(resp) => {
                assert_eq!(resp.id, "300");
                assert_eq!(resp.result["id"], "I99");
                assert_eq!(resp.result["birth"]["date"], "1 JAN 1990");
                assert_eq!(resp.result["birth"]["place"], "Town");
            }
            other => panic!("expected response, got {other:?}"),
        }
    }

    #[test]
    fn create_individual_conflict() {
        let mut base = empty_store();
        base.insert_individual(Individual {
            id: "I1".into(),
            name: None,
            birth: None,
            death: None,
        })
        .unwrap();
        let server = Server::new(Some(base));

        let response = server.handle_request(Request {
            id: "301".into(),
            method: "create_individual".into(),
            params: serde_json::json!({
                "id": "I1",
                "name": "Dup"
            }),
        });

        match response {
            OutboundMessage::Error(err) => {
                assert_eq!(err.id, "301");
                assert_eq!(err.error.code, -32001);
            }
            other => panic!("expected error, got {other:?}"),
        }
    }

    #[test]
    fn creates_family() {
        let server = Server::new(Some(empty_store()));
        let response = server.handle_request(Request {
            id: "400".into(),
            method: "create_family".into(),
            params: serde_json::json!({
                "id": "F9",
                "husband": "I1",
                "wife": "I2",
                "children": ["I3", "I4"]
            }),
        });

        match response {
            OutboundMessage::Response(resp) => {
                assert_eq!(resp.id, "400");
                assert_eq!(resp.result["id"], "F9");
                assert_eq!(resp.result["children"], serde_json::json!(["I3", "I4"]));
            }
            other => panic!("expected response, got {other:?}"),
        }
    }

    #[test]
    fn create_family_conflict() {
        let mut base = empty_store();
        base.insert_family(Family {
            id: "F1".into(),
            husband: None,
            wife: None,
            children: vec![],
        })
        .unwrap();
        let server = Server::new(Some(base));

        let response = server.handle_request(Request {
            id: "401".into(),
            method: "create_family".into(),
            params: serde_json::json!({
                "id": "F1"
            }),
        });

        match response {
            OutboundMessage::Error(err) => {
                assert_eq!(err.id, "401");
                assert_eq!(err.error.code, -32001);
            }
            other => panic!("expected error, got {other:?}"),
        }
    }

    #[test]
    fn create_family_validates_children() {
        let server = Server::new(Some(empty_store()));
        let response = server.handle_request(Request {
            id: "402".into(),
            method: "create_family".into(),
            params: serde_json::json!({
                "id": "F2",
                "children": ["I1", 2]
            }),
        });

        match response {
            OutboundMessage::Error(err) => {
                assert_eq!(err.id, "402");
                assert_eq!(err.error.code, -32602);
            }
            other => panic!("expected error, got {other:?}"),
        }
    }

    #[test]
    fn create_handlers_require_store() {
        let server = Server::default();

        let resp_individual = server.handle_request(Request {
            id: "500".into(),
            method: "create_individual".into(),
            params: serde_json::json!({"id": "I1"}),
        });
        match resp_individual {
            OutboundMessage::Error(err) => assert_eq!(err.error.code, -32000),
            _ => panic!("expected server error"),
        }

        let resp_family = server.handle_request(Request {
            id: "501".into(),
            method: "create_family".into(),
            params: serde_json::json!({"id": "F1"}),
        });
        match resp_family {
            OutboundMessage::Error(err) => assert_eq!(err.error.code, -32000),
            _ => panic!("expected server error"),
        }
    }

    #[test]
    fn create_individual_persists_to_storage() {
        let tmp = tempfile::NamedTempFile::new().expect("temp file");
        let server = Server::with_storage(empty_store(), tmp.path().to_path_buf());

        let response = server.handle_request(Request {
            id: "600".into(),
            method: "create_individual".into(),
            params: serde_json::json!({"id": "I1", "name": "Persisted"}),
        });

        match response {
            OutboundMessage::Response(_) => {}
            other => panic!("expected response, got {other:?}"),
        }

        let contents = std::fs::read_to_string(tmp.path()).expect("read persisted file");
        assert!(contents.contains("I1"));
        assert!(contents.contains("Persisted"));
    }
    #[test]
    fn serves_lines_over_io() {
        let server = Server::new(Some(build_store()));
        let input = r#"
{"id":"1","method":"get_individual","params":{"id":"I1"}}
{"id":"2","method":"get_family","params":{"id":"missing"}}
"#;
        let mut reader = std::io::BufReader::new(std::io::Cursor::new(input));
        let mut output = Vec::new();

        server
            .serve_lines(&mut reader, &mut output)
            .expect("serve should succeed");

        let output_str = String::from_utf8(output).expect("utf8");
        let mut lines = output_str.lines();

        let first: OutboundMessage =
            serde_json::from_str(lines.next().unwrap()).expect("first line parses");
        match first {
            OutboundMessage::Response(resp) => assert_eq!(resp.id, "1"),
            _ => panic!("expected response"),
        }

        let second: OutboundMessage =
            serde_json::from_str(lines.next().unwrap()).expect("second line parses");
        match second {
            OutboundMessage::Error(err) => assert_eq!(err.id, "2"),
            _ => panic!("expected error"),
        }
    }
}
