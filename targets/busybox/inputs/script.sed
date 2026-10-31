s/quick/slow/g
/^$/d
s/\([0-9][0-9]*\)\.\([0-9]*\)/<\2.\1>/g
y/abc/ABC/
3a\
-- appended after line 3 --
$s/$/ [END]/
