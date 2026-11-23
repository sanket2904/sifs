package extract

import (
	"os"
	"testing"
)

func TestIBANValid(t *testing.T) {
	for _, c := range []struct {
		iban string
		want bool
	}{
		{"GB29NWBK60161331926819", true},
		{"GB28NWBK60161331926819", false},
		{"DE89370400440532013000", true},
		{"DE88370400440532013000", false},
		{"NL91ABNA0417164300", true},
		{"FR1420041010050500013M02606", true},
		{"FR1420041010050500013M0260", false}, // one short
		{"ZZ29NWBK60161331926819", false},     // no such country
	} {
		if got := ibanValid(c.iban); got != c.want {
			t.Errorf("ibanValid(%s) = %v, want %v", c.iban, got, c.want)
		}
	}
}

func TestLuhn(t *testing.T) {
	for _, c := range []struct {
		card string
		want bool
	}{
		{"4242424242424242", true},
		{"5555555555554444", true},
		{"4111111111111111", true},
		{"4111111111111112", false},
		{"378282246310005", true},
	} {
		if got := luhn(c.card); got != c.want {
			t.Errorf("luhn(%s) = %v, want %v", c.card, got, c.want)
		}
	}
}

func TestPANValid(t *testing.T) {
	for _, c := range []struct {
		pan  string
		want bool
	}{
		{"ABCPK1234F", true},
		{"ABCPK0000F", false}, // serial 0000 is never issued
		{"ABCDK1234F", false}, // D is no holder status
		{"AAACT1234Z", true},  // company
	} {
		if got := panValid(c.pan); got != c.want {
			t.Errorf("panValid(%s) = %v, want %v", c.pan, got, c.want)
		}
	}
}

func TestMRZValid(t *testing.T) {
	for _, c := range []struct {
		name, l1, l2 string
		want         bool
	}{
		{"ICAO specimen", "P<UTOERIKSSON<<ANNA<MARIA<<<<<<<<<<<<<<<<<<<", "L898902C36UTO7408122F1204159ZE184226B<<<<<10", true},
		{"composite digit wrong", "P<UTOERIKSSON<<ANNA<MARIA<<<<<<<<<<<<<<<<<<<", "L898902C36UTO7408122F1204159ZE184226B<<<<<11", false},
		{"expiry digit wrong", "P<UTOERIKSSON<<ANNA<MARIA<<<<<<<<<<<<<<<<<<<", "L898902C36UTO7408122F1204158ZE184226B<<<<<10", false},
		{"visa MRV-A", "V<UTOERIKSSON<<ANNA<MARIA<<<<<<<<<<<<<<<<<<<", "L8988901C4XXX4009078F96121096ZE184226B<<<<<<", true},
		{"short line", "P<UTO", "L898902C3", false},
	} {
		if got := mrzValid(c.l1, c.l2); got != c.want {
			t.Errorf("%s: mrzValid = %v, want %v", c.name, got, c.want)
		}
	}
}

func TestRIBKey(t *testing.T) {
	for _, c := range []struct {
		bank, branch, account, want string
	}{
		{"20041", "01005", "0500013M026", "06"},
		{"30002", "00550", "0000157841Z", "25"},
	} {
		if got, ok := ribKey(c.bank, c.branch, c.account); !ok || got != c.want {
			t.Errorf("ribKey(%s %s %s) = %s, want %s", c.bank, c.branch, c.account, got, c.want)
		}
	}
}

func TestMatchKey(t *testing.T) {
	for _, c := range []struct{ in, want string }{
		{"HDFC Bank Ltd", "hdfcbank"},
		{"HDFC BANK", "hdfcbank"},
		{"hdfcbank", "hdfcbank"},
		{"The Bank of Example plc", "bankofexample"},
		{"三菱 UFJ 銀行", "三菱ufj銀行"},
		{"株式会社三菱ＵＦＪ銀行", "三菱ufj銀行"},
		{"Punjab & Sind Bank", "punjabsindbank"},
	} {
		if got := matchKey(c.in); got != c.want {
			t.Errorf("matchKey(%q) = %q, want %q", c.in, got, c.want)
		}
	}
}

// The directory stores each name's match key. Go has no NFKC in its
// standard library, so this measures how far the partial fold goes.
func TestMatchKeyAgreesWithDirectory(t *testing.T) {
	if _, err := os.Stat("../../sample/directory/banks.json"); err != nil {
		t.Skip("no directory beside the module")
	}
	d, err := LoadDirectory("../../sample/directory/banks.json")
	if err != nil {
		t.Fatal(err)
	}
	total, differ := 0, 0
	for _, e := range d.entries {
		for _, n := range e.Names {
			total++
			if matchKey(n.Text) != n.Key {
				differ++
				t.Logf("%q: ours %q, directory %q", n.Text, matchKey(n.Text), n.Key)
			}
		}
	}
	if differ*100 > total {
		t.Errorf("%d of %d directory names get another key", differ, total)
	}
}
