// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package composition

import "github.com/TokenNotIncluded/lmm-api/extensions/internal/coreclient"

func ioNetKey(cfg Config, credentials ...[]byte) ([]byte, error) {
	// Reuse APP01's loaded-material isolation checks, not its encryption purpose.
	// Check the original repository key too before selecting the io.net file.
	if cfg.ScriptRepositoryKeyFile != "" {
		other, err := coreclient.ReadPrivateFile(cfg.ScriptRepositoryKeyFile, 32)
		if err != nil {
			return nil, err
		}
		credentials = append(credentials, other)
	}
	copy := cfg
	copy.ScriptRepositoryKeyFile = cfg.IoNetKeyFile
	return scriptRepositoryKey(copy, credentials...)
}
