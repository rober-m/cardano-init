# Devnet — Dingo

[Dingo](https://github.com/blinklabs-io/dingo) runs a private, single-node
Cardano devnet with a Blockfrost-compatible API. Docker Compose pins Dingo
0.69.0 by OCI digest for Linux amd64 and arm64.

## Tasks

| Command | Action |
|---|---|
| `just test` | Start an isolated chain, verify Blockfrost, submit a faucet transaction, verify its UTxO, and tear down. |
| `just dev` | Start a fresh persistent devnet and write its API URL to `../.env`. |
| `just fund ADDRESS [LOVELACE]` | Fund a testnet address; default 100 ADA. |
| `just faucet-address` | Print the bundled faucet address. |
| `just clean` | Remove containers and volumes and clear Dingo's `.env` values. |

The API base is `http://localhost:3000/api/v0/`. Override the host port with
`DINGO_DEVNET_BLOCKFROST_PORT` before running `just dev` or `just test`.

`just dev` creates a new data volume each time. The startup script copies
Dingo's bundled devnet configuration and sets both genesis start times to the
current UTC time before the node starts.

The faucet credentials are public test fixtures bundled in the Dingo image.
They are for this disposable devnet only.
