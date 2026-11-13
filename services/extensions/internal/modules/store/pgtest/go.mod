module github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/store/pgtest

go 1.26.0

require (
	github.com/TokenNotIncluded/lmm-api/extensions v0.0.0
	github.com/lib/pq v1.12.3
)

require (
	github.com/BurntSushi/toml v1.6.0 // indirect
	golang.org/x/crypto v0.59.0 // indirect
)

replace github.com/TokenNotIncluded/lmm-api/extensions => ../../../..
