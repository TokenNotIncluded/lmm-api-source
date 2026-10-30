module github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/payments/pgtest

go 1.25.0

require (
	github.com/TokenNotIncluded/lmm-api/extensions v0.0.0
	github.com/lib/pq v1.10.9
)

require (
	github.com/waffo-com/waffo-go/v2 v2.1.0 // indirect
	github.com/waffo-com/waffo-pancake-sdk-go v0.17.0 // indirect
)

replace github.com/TokenNotIncluded/lmm-api/extensions => ../../../..
