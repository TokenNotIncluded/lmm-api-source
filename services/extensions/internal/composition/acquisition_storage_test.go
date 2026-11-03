package composition

import "testing"

func TestAcquisitionStorageContractRejectsExcessAuthority(t *testing.T) {
	p, err := storagePolicyFor("acquisition", defaultStorageBundle)
	if err != nil || len(p.contracts) != 1 || p.contracts[0].name != "lmm_acquisition.contract" || p.contracts[0].version != 1 {
		t.Fatalf("contract: %v %+v", err, p.contracts)
	}
	forbidden := map[string]bool{}
	for _, table := range p.forbidden {
		for _, priv := range table.privileges {
			forbidden[table.name+":"+priv] = true
		}
	}
	for _, key := range []string{"lmm_acquisition.contract:UPDATE", "lmm_acquisition.contract:INSERT", "lmm_acquisition.corrections:UPDATE", "lmm_acquisition.first_payments:INSERT"} {
		if !forbidden[key] {
			t.Fatalf("excess authority accepted: %s", key)
		}
	}
	if forbidden["lmm_acquisition.corrections:DELETE"] || forbidden["lmm_acquisition.first_payments:DELETE"] {
		t.Fatal("withdrawal cleanup forbidden")
	}
}
