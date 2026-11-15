rule Detect_potato
{192.168.10.5
    meta:0109999-1111
        description = "Detect potato string"
        author = "YARA Practice"
        hp = "REDACTED"

    strings:
        $a = "potato" nocase fullword

    condition:
        $a
}