package extract

import (
	"math/big"
	"strings"
)

// ibanCountry is one row of the SWIFT IBAN registry: the IBAN's length and,
// where the registry gives them, where the bank code, branch code and
// account number sit (offsets in the IBAN, end exclusive; zero when absent).
type ibanCountry struct {
	length             int
	bank0, bank1       int
	branch0, branch1   int
	account0, account1 int
}

var ibanCountries = map[string]ibanCountry{
	"AD": {length: 24}, "AE": {length: 23}, "AL": {length: 28}, "AZ": {length: 28},
	"BA": {length: 20}, "BG": {length: 22}, "BH": {length: 22}, "BR": {length: 29},
	"CR": {length: 22}, "CY": {length: 28}, "CZ": {length: 24}, "DK": {length: 18},
	"DO": {length: 28}, "EE": {length: 20}, "EG": {length: 29}, "FI": {length: 18},
	"FO": {length: 18}, "GE": {length: 22}, "GI": {length: 23}, "GL": {length: 18},
	"GR": {length: 27}, "GT": {length: 28}, "HR": {length: 21}, "HU": {length: 28},
	"IL": {length: 23}, "IS": {length: 26}, "IT": {length: 27}, "JO": {length: 30},
	"KW": {length: 30}, "KZ": {length: 20}, "LB": {length: 28}, "LC": {length: 32},
	"LI": {length: 21}, "LT": {length: 20}, "LV": {length: 21}, "MD": {length: 24},
	"ME": {length: 22}, "MK": {length: 19}, "MR": {length: 27}, "MT": {length: 31},
	"MU": {length: 30}, "NO": {length: 15}, "PK": {length: 24}, "PL": {length: 28},
	"PS": {length: 29}, "PT": {length: 25}, "QA": {length: 29}, "RO": {length: 24},
	"RS": {length: 22}, "SA": {length: 24}, "SE": {length: 24}, "SI": {length: 19},
	"SK": {length: 24}, "SM": {length: 27}, "TN": {length: 24}, "TR": {length: 26},
	"UA": {length: 29}, "VG": {length: 24}, "XK": {length: 20},
	"AT": {length: 20, bank0: 4, bank1: 9, account0: 9, account1: 20},
	"BE": {length: 16, bank0: 4, bank1: 7, account0: 7, account1: 14},
	"CH": {length: 21, bank0: 4, bank1: 9, account0: 9, account1: 21},
	"DE": {length: 22, bank0: 4, bank1: 12, account0: 12, account1: 22},
	"ES": {length: 24, bank0: 4, bank1: 8, branch0: 8, branch1: 12, account0: 14, account1: 24},
	"FR": {length: 27, bank0: 4, bank1: 9, branch0: 9, branch1: 14, account0: 14, account1: 25},
	"MC": {length: 27, bank0: 4, bank1: 9, branch0: 9, branch1: 14, account0: 14, account1: 25},
	"GB": {length: 22, bank0: 4, bank1: 8, branch0: 8, branch1: 14, account0: 14, account1: 22},
	"IE": {length: 22, bank0: 4, bank1: 8, branch0: 8, branch1: 14, account0: 14, account1: 22},
	"LU": {length: 20, bank0: 4, bank1: 7, account0: 7, account1: 20},
	"NL": {length: 18, bank0: 4, bank1: 8, account0: 8, account1: 18},
}

// ibanValid applies the ISO 13616 check: move the first four characters to
// the end, turn letters into 10..35, and the number mod 97 must be 1.
func ibanValid(iban string) bool {
	c, ok := ibanCountries[iban[:2]]
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

func mrzCheck(field string, digit byte) bool {
	if digit == '<' {
		digit = '0'
	}
	return mrzDigit(field) == digit
}

// mrzValid checks every check digit of a TD3 passport zone (P) or an MRV-A
// visa zone (V); both have two lines of 44 characters.
func mrzValid(l1, l2 string) bool {
	if len(l1) != 44 || len(l2) != 44 {
		return false
	}
	ok := mrzCheck(l2[0:9], l2[9]) && mrzCheck(l2[13:19], l2[19]) && mrzCheck(l2[21:27], l2[27])
	if l1[0] == 'P' {
		ok = ok && mrzCheck(l2[28:42], l2[42]) && mrzCheck(l2[0:10]+l2[13:20]+l2[21:43], l2[43])
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
