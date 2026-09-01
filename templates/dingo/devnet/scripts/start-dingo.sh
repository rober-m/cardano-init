#!/bin/sh
# Create a fresh timestamped genesis from Dingo's bundled devnet configuration.
set -eu

config_dir=/data/db/devnet-config

if [ ! -f "$config_dir/config.json" ]; then
  mkdir -p "$config_dir"
  cp -R /opt/cardano/config/devnet/. "$config_dir/"

  now=$(date -u +%s)
  now_iso=$(date -u -d "@$now" +%Y-%m-%dT%H:%M:%SZ)
  sed -i -E "s/(\"startTime\": )[0-9]+/\1$now/" "$config_dir/byron-genesis.json"
  sed -i -E "s/(\"systemStart\": \")[^\"]+/\1$now_iso/" "$config_dir/shelley-genesis.json"

  # Slow the epoch down. Dingo images up to and including 0.70.2 bundle a devnet
  # genesis with a 5 slot epoch at 0.1 second slots, so the chain crosses an
  # epoch boundary twice a second. After a few hundred epochs GetStakePools
  # stops answering, and cardano-cli issues that query while balancing, so
  # `just fund` fails; on 0.70.2 the ledger tip also stops advancing while the
  # forger keeps going. That genesis is fixed upstream in
  # blinklabs-io/dingo#3636 and blinklabs-io/docker-cardano-configs#79; drop
  # this block once the pinned image carries the fix.
  #
  # The values match what upstream now ships: 1 second slots, a 600 slot
  # (10 minute) epoch, and securityParam 100, which keeps the Conway
  # randomness stabilisation window (4k/f = 400) inside the epoch. Byron k 60
  # puts the Byron epoch (10k slots) at the same 600.
  sed -i -E \
    -e 's/("epochLength": )[0-9]+/\1600/' \
    -e 's/("slotLength": )[0-9.]+/\11/' \
    -e 's/("securityParam": )[0-9]+/\1100/' \
    "$config_dir/shelley-genesis.json"
  sed -i -E \
    -e 's/("k": )[0-9]+/\160/' \
    -e 's/("slotDuration": ")[0-9]+"/\11000"/' \
    "$config_dir/byron-genesis.json"
fi

exec /bin/entrypoint.sh serve
