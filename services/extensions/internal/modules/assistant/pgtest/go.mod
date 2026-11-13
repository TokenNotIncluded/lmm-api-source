module github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/pgtest

go 1.26.0

require (
	github.com/TokenNotIncluded/lmm-api/extensions v0.0.0
	github.com/lib/pq v1.12.3
)

require (
	google.golang.org/grpc v1.85.0 // indirect
	google.golang.org/protobuf v1.36.12 // indirect
)

require (
	golang.org/x/net v0.62.0 // indirect
	golang.org/x/sys v0.50.0 // indirect
	golang.org/x/text v0.44.0 // indirect
	google.golang.org/genproto/googleapis/rpc v0.0.0-20261110000943-09e21b237f2c // indirect
)

replace github.com/TokenNotIncluded/lmm-api/extensions => ../../../..
