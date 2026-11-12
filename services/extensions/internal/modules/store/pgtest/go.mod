module github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/store/pgtest

go 1.25.0

require (
	github.com/TokenNotIncluded/lmm-api/extensions v0.0.0
	github.com/lib/pq v1.10.9
)

require (
	github.com/BurntSushi/toml v1.6.0 // indirect
	golang.org/x/crypto v0.54.0 // indirect
)

replace github.com/TokenNotIncluded/lmm-api/extensions => ../../../..
