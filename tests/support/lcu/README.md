# Synthetic LCU fixtures

session.json is generated test data, not a captured ranked session. Its consumed fields follow the immutable 16.19 Hasagi contract pinned in [lcu-contract-pin.json](lcu-contract-pin.json). Nested actions and ordering remain compatibility assumptions; tests also exercise flat actions. Identity/chat fields are deliberately omitted or empty. No token, names, PUUIDs or real-player records are included.

Download the immutable schema URL from the pin separately, then run
`python3 tests/support/lcu/verify_contract.py PATH_TO_SCHEMA`. This validates
SHA-256 and consumed fields, not live events or current client compatibility.
