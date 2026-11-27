package postgresurl

import "os"

// Keep the list in sync with lib/pq v1.10.9 parseEnviron, plus PGPASSFILE.
// Both URL profiles reject even empty values before lib/pq parses the URL.
func hasConnectionEnvironment() bool {
	for _, name := range []string{
		"PGHOST", "PGHOSTADDR", "PGPORT", "PGDATABASE", "PGUSER",
		"PGPASSWORD", "PGSERVICE", "PGSERVICEFILE", "PGREALM", "PGOPTIONS",
		"PGAPPNAME", "PGSSLMODE", "PGSSLCERT", "PGSSLKEY", "PGSSLROOTCERT",
		"PGSSLSNI", "PGREQUIRESSL", "PGSSLCRL", "PGREQUIREPEER", "PGKRBSRVNAME",
		"PGGSSLIB", "PGCONNECT_TIMEOUT", "PGCLIENTENCODING", "PGDATESTYLE", "PGTZ",
		"PGGEQO", "PGSYSCONFDIR", "PGLOCALEDIR", "PGPASSFILE",
	} {
		if _, present := os.LookupEnv(name); present {
			return true
		}
	}
	return false
}
