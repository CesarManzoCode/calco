BEGIN { FS = "," }
NR > 1 {
	total += $3
	prod = $3 * $4
	printf "%-8s %5d %10.4f %s\n", $2, $3, prod, toupper(substr($2, 1, 3))
	if ($3 > max) { max = $3; best = $2 }
}
END {
	printf "count=%d total=%d mean=%.6f best=%s\n", NR - 1, total, total / (NR - 1), best
	printf "sqrt2=%.10f exp1=%.10f log10=%.10f sin1=%.10f\n", sqrt(2), exp(1), log(10), sin(1)
	printf "int=%d mod=%d pow=%g\n", int(-7.9), 17 % 5, 2 ^ 40
}
