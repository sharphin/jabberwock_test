function numeric_row(label) {
    if (NF != 7) { print "ERROR: invalid field count in " label ": " $0; bad = 1; return 0 }
    for (k = 2; k <= 7; k++)
        if ($k !~ /^[0-9]+$/) { print "ERROR: invalid numeric field in " label ": " $0; bad = 1; return 0 }
    return 1
}
FILENAME == ARGV[1] {
    if ($1 ~ /^#/ || NF == 0) next
    if (!numeric_row("baseline")) next
    if (seen_base[$1]++) { print "ERROR: duplicate baseline " $1; bad = 1 }
    base[$1] = $0
    base_count++
    next
}
{
    if (!numeric_row("current")) next
    if (seen_now[$1]++) { print "ERROR: duplicate current " $1; bad = 1; next }
    if (!($1 in base)) { print "ERROR: unexpected case " $1; bad = 1; next }
    split(base[$1], b, " ")
    if ($2 > b[2] || $4 > b[4] || $5 > b[5] || $7 > b[7] || $3 < b[3] || $6 < b[6]) {
        print "!! " $1 ": baseline [" base[$1] "] current [" $0 "]"
        bad = 1
    } else print "ok " $0
}
END {
    for (name in base) if (!(name in seen_now)) { print "ERROR: missing case " name; bad = 1 }
    if (base_count == 0) { print "ERROR: empty baseline"; bad = 1 }
    exit bad
}
