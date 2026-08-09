# MMX poker dealer

`poker-dealer` is the off-chain coordinator for `poker2.js`. It uses an MMX
node's authenticated WAPI for chain reads, table deployment, and dealer
settlement transactions. Players use a read-only HTTP/JSON API and one
authenticated WebSocket connection.

The dealer tracks confirmed table snapshots in the Stainless versioned
kvstore. Each observed chain tip is a store version; a node reorganization
walks the saved checkpoints to the newest common ancestor and reverts the
store before resynchronizing. Pending deployments and in-progress hand state
are durable in separate logs.

The dealer is implemented in Stainless. `protocol.stl` owns the
consensus-facing `poker2.js` hashes; `model.stl` defines the typed internal
state and its associated `parse()`, `json()`, and `json_public()` boundary
conversions; `poker.stl` owns the player API, authentication, WAPI calls,
rollback handling, table discovery, and scaling; `hand.stl` owns the hand
state machine, transcript checkpoints, timeouts, showdown, side pots/rake,
and settlement payload; and `main.stl` owns the server event loop.

## Running

Copy `poker.example.json` to `poker.json`, then replace `poker_binary`,
`dealer`, and `api_token`. The dealer is a standalone Stainless program; it
does not contain a Cargo package, `build.rs`, `main.rs`, or any other Rust
application source.

From the Stainless workspace root, build it with:

```sh
stainlessc --build --package apps/poker -o poker-dealer
```

`apps/poker/stainless-package.toml` declares the Stainless dependencies and
entry point. Each dependency owns its own source list, Rust bindings, and any
native Cargo dependency, so the poker build does not enumerate or build
dependency sources itself.

Run `poker-dealer` from the directory containing `poker.json`.

The dealer tests are themselves a standalone Stainless program:

```sh
stainlessc --run --package apps/poker/test
```

`poker_binary` is the already-deployed `poker2.js` binary address. The dealer
deploys executable table instances; it does not deploy the binary. `dealer`
must be an address controlled by `wallet_index`. The WAPI token needs spending
permission and that wallet must be available for unattended signing. Poker
adds the token to its HTTP `ClientConfig` as the MMX-specific `x-api-token`
header; the generic HTTP client has no MMX authentication policy.

Template fields map directly to the contract initializer. `start_delay` and
`game_timeout` are block counts. `player_timeout` is seconds and is also stored
in the contract so clients can distinguish fast and slow tables. For every
template, `min_available` is the desired count of confirmed open tables with a
free next-hand seat, plus pending deployments. At most
`max_deploy_per_sync` instances are created during one node synchronization.

The example addresses are syntactically valid placeholders; do not run the
example unchanged.

Values whose model type is `u64` or `u128` are decimal strings in configuration
and in the player HTTP/WebSocket protocol. Bounded `u32` fields such as
`wallet_index`, `max_players`, and `rake_bps` remain JSON numbers. Contract
arguments are converted to lowercase `0x` hexadecimal strings before WAPI
submission.

WebSocket enum values use their declared Stainless member names, matching VNX
enum-to-string conversion. They are case-sensitive: for example, `Commit`,
`HandState`, and `Settling`.

## Player API

All REST responses are JSON:

- `GET /v1/health` returns node-tip and table counts.
- `GET /v1/templates` returns public dealer configuration and table classes.
- `GET /v1/tables` returns confirmed table snapshots.
- `GET /v1/tables/{address}` returns one tracked table.
- `GET /v1/openapi.json` returns the REST description.
- `GET /v1/ws` upgrades to the player WebSocket protocol.

Joining, topping up, activating, deactivating, claiming, and emergency refunds
remain direct on-chain player operations. After joining, a player connects to
the dealer and authenticates ownership of the same key.

### WebSocket authentication

The server first sends:

```json
{"type":"Challenge","protocol_version":1,"dealer":"mmx1...","challenge":"32-byte-hex","expires_at":"1786000000"}
```

The player signs the SHA-256 hash of:

```text
MMX_POKER_DEALER_AUTH_V1/{dealer}/{challenge}/{expires_at}/{address}
```

and replies:

```json
{"type":"Auth","address":"mmx1...","public_key":"33-byte-compressed-hex","signature":"64-byte-compact-hex"}
```

After `Authenticated`, subscribe with
`{"type":"Subscribe","table":"mmx1..."}`. The player must already be
registered in that table contract. The dealer sends `HandState` whenever a
hand starts or changes phase. It includes the exact `checkpoint`, deadline,
roster stacks/bets/folds, public transcript collected so far, board when known,
and computed result stacks before continuation.

### Hand submissions

Every submission includes `type`, `hand_id`, `round`, and `epoch`. Signatures
are compact 64-byte secp256k1 signatures. Hash construction is available from
the poker package's `poker::protocol` namespace and is byte-for-byte aligned
with `poker2.js`.

- Commit: `{"type":"Commit","hand_id":"7","round":"0","epoch":"0","commitments":["..."],"signature":"..."}`
- Reveal: `{"type":"Reveal","hand_id":"7","round":"0","epoch":"0","seed":"32-byte-hex"}`
- Action: `{"type":"Action","hand_id":"7","round":"0","epoch":"0","action":"BetRaise","cumulative_bet":"2000","checkpoint":"...","signature":"..."}`
- Show: `{"type":"Show","hand_id":"7","round":"4","epoch":"0","pocket_seed":"32-byte-hex","hand":[0,1,2,3,4]}`
- Muck immediately: `{"type":"Muck","hand_id":"7","round":"4","epoch":"0"}`
- Continue: `{"type":"Continue","hand_id":"7","round":"0","epoch":"0","result_stack":"3100","checkpoint":"...","signature":"..."}`
- Leave immediately: `{"type":"Leave","hand_id":"7","round":"0","epoch":"0"}`

Commit, action, and continuation signatures use the corresponding canonical
contract hashes. Reveals and shows are authenticated by the WebSocket session
and checked against their earlier seed commitments. The dealer rejects stale
hand/round/epoch/checkpoint messages before placing them in the hand inbox.
Missing submissions become the timeout records defined by the contract.
Stack, bet, rake, and payout fields are handled as exact `u128` values and are
emitted as decimal strings in `HandState`. All `u64`/`u128` player inputs are
parsed exactly, without floating-point coercion. Every integer passed to an MMX
contract is encoded as a hexadecimal string.

Betting follows the contract's parallel epoch model: every eligible player in
an epoch signs the same starting checkpoint. A raise that leaves another live
player below the new target opens the next epoch. Settlement is submitted only
after payouts are reproduced off-chain and continuation responses or timeouts
are complete.
