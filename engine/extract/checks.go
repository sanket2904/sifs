package extract

import (
	"encoding/json"
	"fmt"
	"math/big"
	"os"
	"strings"

	"github.com/sanket2904/sifs/engine/extract/orgkey"
)

// IBANRegistry is sample/iban-registry.json, the table the scorer reads:
// each registry country's IBAN length and where its bank identifier sits.
type IBANRegistry struct{ countries map[string]ibanCountry }

// ibanCountry gives an IBAN's length and its bank identifier's place,
// offsets in the IBAN, end exclusive.
type ibanCountry struct{ length, bank0, bank1 int }

// LoadIBANRegistry reads the registry table. Every country whose IBANs the
// extractor looks for must be in it.
func LoadIBANRegistry(path string) (*IBANRegistry, error) {
	raw, err := os.ReadFile(path)
	if err != nil {
		return nil, err
	}
	var f struct {
		Countries map[string]struct {
			Length int    `json:"length"`
			Bank   string `json:"bank_identifier"`
		} `json:"countries"`
	}
	if err := json.Unmarshal(raw, &f); err != nil {
		return nil, fmt.Errorf("%s: %w", path, err)
	}
	reg := &IBANRegistry{countries: map[string]ibanCountry{}}
	for cc, c := range f.Countries {
		var first, last int
		if n, _ := fmt.Sscanf(c.Bank, "%d-%d", &first, &last); n != 2 || first < 1 || last < first || 4+last > c.Length {
			return nil, fmt.Errorf("%s: %s: bank_identifier %q is no place in an IBAN of %d characters", path, cc, c.Bank, c.Length)
		}
		reg.countries[cc] = ibanCountry{length: c.Length, bank0: 4 + first - 1, bank1: 4 + last}
	}
	for cc := range ibanSought {
		if _, ok := reg.countries[cc]; !ok {
			return nil, fmt.Errorf("%s: no row for %s", path, cc)
		}
	}
	return reg, nil
}

// ibanSought are the countries whose IBANs findIBAN looks for.
var ibanSought = set(strings.Fields(`AD AE AL AT AZ BA BE BG BH BR CH CR CY CZ DE DK DO EE EG ES FI FO FR GB GE GI GL GR GT
HR HU IE IL IS IT JO KW KZ LB LC LI LT LU LV MC MD ME MK MR MT MU NL NO PK PL PS PT QA RO RS SA SE SI SK SM TN TR UA VG XK`)...)

// ibanParts are the countries whose IBAN bank code the extractor reads, with
// where the branch code and the account number sit in their IBANs (offsets
// in the IBAN, end exclusive; zero when absent). The registry table holds
// neither place.
var ibanParts = map[string]struct{ branch0, branch1, account0, account1 int }{
	"AT": {account0: 9, account1: 20},
	"BE": {account0: 7, account1: 14},
	"CH": {account0: 9, account1: 21},
	"DE": {account0: 12, account1: 22},
	"ES": {branch0: 8, branch1: 12, account0: 14, account1: 24},
	"FR": {branch0: 9, branch1: 14, account0: 14, account1: 25},
	"MC": {branch0: 9, branch1: 14, account0: 14, account1: 25},
	"GB": {branch0: 8, branch1: 14, account0: 14, account1: 22},
	"IE": {branch0: 8, branch1: 14, account0: 14, account1: 22},
	"LU": {account0: 7, account1: 20},
	"NL": {account0: 8, account1: 18},
}

// proves applies FORMAT.md's test for a certain IBAN: its country is in the
// table, it has that country's length, and the ISO 13616 check passes (move
// the first four characters to the end, turn letters into 10..35, and the
// number mod 97 must be 1).
func (reg *IBANRegistry) proves(iban string) bool {
	c, ok := reg.countries[iban[:2]]
	if !ok || len(iban) != c.length {
		return false
	}
	var b strings.Builder
	for _, r := range iban[4:] + iban[:4] {
		switch {
		case r >= '0' && r <= '9':
			b.WriteRune(r)
		case r >= 'A' && r <= 'Z':
			b.WriteString(itoa(int(r-'A') + 10))
		default:
			return false
		}
	}
	n, ok := new(big.Int).SetString(b.String(), 10)
	return ok && new(big.Int).Mod(n, big.NewInt(97)).Int64() == 1
}

// codeKey is the key of the bank an IBAN names by its bank code ("Code
// keys"), for a country whose bank code the extractor reads, or zero.
func (reg *IBANRegistry) codeKey(iban string) orgkey.Key {
	if _, ok := ibanParts[iban[:2]]; !ok {
		return orgkey.Key{}
	}
	c := reg.countries[iban[:2]]
	return orgkey.IBAN(iban[:2], iban[c.bank0:c.bank1])
}

func itoa(n int) string {
	if n == 0 {
		return "0"
	}
	var d []byte
	for ; n > 0; n /= 10 {
		d = append([]byte{byte('0' + n%10)}, d...)
	}
	return string(d)
}

// luhn is the card scheme's check over the digits of s.
func luhn(s string) bool {
	sum, double := 0, false
	for i := len(s) - 1; i >= 0; i-- {
		d := int(s[i] - '0')
		if double {
			d *= 2
			if d > 9 {
				d -= 9
			}
		}
		sum += d
		double = !double
	}
	return len(s) > 0 && sum%10 == 0
}

// panValid is the Income Tax Department's form: five letters, four digits,
// a letter; the fourth letter is a holder status; serial 0000 is never issued.
func panValid(p string) bool {
	return strings.ContainsRune("ABCFGHLJPT", rune(p[3])) && p[5:9] != "0000"
}

// mrzDigit is ICAO 9303's check digit with weights 7, 3, 1.
func mrzDigit(s string) byte {
	w := [3]int{7, 3, 1}
	sum := 0
	for i := 0; i < len(s); i++ {
		c := s[i]
		v := 0
		switch {
		case c >= '0' && c <= '9':
			v = int(c - '0')
		case c >= 'A' && c <= 'Z':
			v = int(c-'A') + 10
		}
		sum += v * w[i%3]
	}
	return byte('0' + sum%10)
}

func mrzCheck(field string, digit byte) bool { return mrzDigit(field) == digit }

// mrzValid checks every check digit of a TD3 passport zone (P) or an MRV-A
// visa zone (V); both have two lines of 44 characters. A check digit is a
// digit, except that an empty personal number may give a filler.
func mrzValid(l1, l2 string) bool {
	if len(l1) != 44 || len(l2) != 44 {
		return false
	}
	ok := mrzCheck(l2[0:9], l2[9]) && mrzCheck(l2[13:19], l2[19]) && mrzCheck(l2[21:27], l2[27])
	if l1[0] == 'P' {
		personal := mrzCheck(l2[28:42], l2[42]) || l2[42] == '<' && strings.Trim(l2[28:42], "<") == ""
		ok = ok && personal && mrzCheck(l2[0:10]+l2[13:20]+l2[21:43], l2[43])
	}
	return ok
}

// ribKey recomputes the French clé RIB from bank, branch and account codes.
func ribKey(bank, branch, account string) (string, bool) {
	var b strings.Builder
	for _, r := range strings.ToUpper(account) {
		switch {
		case r >= '0' && r <= '9':
			b.WriteRune(r)
		case r >= 'A' && r <= 'Z':
			b.WriteByte("12345678912345678923456789"[r-'A']) // A,J=1 B,K,S=2 ... I,R,Z=9
		default:
			return "", false
		}
	}
	n, ok := new(big.Int).SetString(bank+branch+b.String(), 10)
	if !ok {
		return "", false
	}
	k := 97 - new(big.Int).Mod(n.Mul(n, big.NewInt(100)), big.NewInt(97)).Int64()
	s := itoa(int(k))
	if len(s) == 1 {
		s = "0" + s
	}
	return s, true
}

// isoCountries holds the ISO 3166-1 alpha-2 codes a BIC's country part may take.
var isoCountries = func() map[string]bool {
	m := map[string]bool{}
	for _, c := range strings.Fields(`AD AE AF AG AI AL AM AO AQ AR AS AT AU AW AX AZ BA BB BD BE BF BG BH BI BJ BL BM BN BO BQ BR BS BT BV BW BY BZ
CA CC CD CF CG CH CI CK CL CM CN CO CR CU CV CW CX CY CZ DE DJ DK DM DO DZ EC EE EG EH ER ES ET FI FJ FK FM FO FR GA GB GD GE GF GG GH GI GL GM GN GP GQ GR GS GT GU GW GY
HK HM HN HR HT HU ID IE IL IM IN IO IQ IR IS IT JE JM JO JP KE KG KH KI KM KN KP KR KW KY KZ LA LB LC LI LK LR LS LT LU LV LY MA MC MD ME MF MG MH MK ML MM MN MO MP MQ MR MS MT MU MV MW MX MY MZ
NA NC NE NF NG NI NL NO NP NR NU NZ OM PA PE PF PG PH PK PL PM PN PR PS PT PW PY QA RE RO RS RU RW SA SB SC SD SE SG SH SI SJ SK SL SM SN SO SR SS ST SV SX SY SZ
TC TD TF TG TH TJ TK TL TM TN TO TR TT TV TW TZ UA UG UM US UY UZ VA VC VE VG VI VN VU WF WS XK YE YT ZA ZM ZW`) {
		m[c] = true
	}
	return m
}()
