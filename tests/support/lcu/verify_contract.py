"""Check the pinned extracted schema; this does not validate live event ordering."""
import hashlib
import json
from pathlib import Path
import sys

def verify(schema_path):
    pin = json.loads(Path(__file__).with_name("lcu-contract-pin.json").read_text())
    payload = Path(schema_path).read_bytes()
    if hashlib.sha256(payload).hexdigest() != pin["sha256"]:
        raise ValueError("Different source bytes: review and repin deliberately")
    schema = json.loads(payload)
    if schema["info"]["version"] != pin["client_version"]:
        raise ValueError("Different client version")
    response = schema["paths"][pin["read_endpoint"]]["get"]["responses"]["2XX"]
    ref = response["content"]["application/json"]["schema"]["$ref"]
    if ref != "#/components/schemas/" + pin["response_schema"]:
        raise ValueError("Different session response model")
    count = 0
    for model, fields in pin["required_consumed_fields"].items():
        properties = schema["components"]["schemas"][model]["properties"]
        for name, expected in fields.items():
            value = properties[name]
            actual = "reference" if "$ref" in value else value.get("type")
            if actual != expected:
                raise ValueError(f"Different field type: {model}.{name}")
            count += 1
    return {"client_version": pin["client_version"], "verified_fields": count,
            "live_events_validated": False}

if __name__ == "__main__":
    print(json.dumps(verify(sys.argv[1]), indent=2))
