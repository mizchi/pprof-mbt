package demangle

import "testing"

func TestSymbol(t *testing.T) {
	cases := []struct {
		in, want string
	}{
		{"_M0FP26mizchi5bench9ackermann", "mizchi::bench::ackermann"},
		{"_M0FP26mizchi5bench11mandel__sum", "mizchi::bench::mandel__sum"},
		{"__M0FP26mizchi5bench3fib", "mizchi::bench::fib"},
		{"M0FP26mizchi5bench9ackermann", "mizchi::bench::ackermann"},
		{"_M0FPB7printlnGsE", "println"},
		{"main", "main"},
		{"", ""},
		{"printc", "printc"},
		// Malformed input shouldn't infinite-loop.
		{"_M0FPXXXXX", "_M0FPXXXXX"},
	}
	for _, c := range cases {
		got := Symbol(c.in)
		if got != c.want {
			t.Errorf("Symbol(%q) = %q, want %q", c.in, got, c.want)
		}
	}
}

func BenchmarkSymbolUserFunction(b *testing.B) {
	for i := 0; i < b.N; i++ {
		_ = Symbol("_M0FP26mizchi5bench9ackermann")
	}
}
