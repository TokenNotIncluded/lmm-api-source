# Core console configuration

Core reads its host configuration at startup. The default main file is `/etc/lmm.conf.d/core.toml`. Keep secrets in the existing private credential files referenced by that configuration.

## Change the effective configuration

For a main file named `core.toml`, put optional overrides in the adjacent `core.toml.d/` directory. Core loads the main file first, then the visible `*.toml` fragments in filename order. Later scalar values and arrays replace earlier ones. Nested tables merge by key. An existing main file works without a drop-in directory. An explicit `--config /absolute/directory` uses the TOML fragments in that directory as the complete configuration.

For example, on a host with the console already enabled, `/etc/lmm.conf.d/core.toml.d/50-console.toml` can contain:

```toml
[console]
notice = "Scheduled maintenance on Sunday, 02:00–02:15 UTC."
usd_exchange_rate = "7.25"
```

Restart Core through the site's usual service manager to load startup configuration changes. The exchange rate controls display conversion. The running service reads the configured notice for the public console. For settings with existing database records, an explicit file value takes precedence. Omitting it preserves the existing read-only fallback. Use an explicit empty notice to clear a notice inherited from an earlier installation.

## Inspect settings in the browser

Open **System Settings** to inspect the running site. Root sessions can also read the bounded projection at `GET /api/option/`. Its capabilities include `read_only: true` and an empty `writable_keys` list. The former single and bulk write routes reject changes with HTTP 405. Host credentials and private file references stay on the server.

Open **Plugin management** to change settings owned by an installed plugin. The plugin's own authorization and validation apply. For HeroSMS, Core owns the enabled flags and price multiplier in the `[hero_sms].policy_file` JSON file. For login provider administration, see [Federated authentication](federated-auth.md).

Existing database rows, revision metadata and historical audit records retain their migration and recovery contracts. See the [network access guide](network-access.md) for IP policy semantics.
