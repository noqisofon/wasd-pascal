PROGRAM WriteLnDemo;

VAR
    total: INTEGER;
    i: INTEGER;
    isPositive: BOOLEAN;

BEGIN
    total := 0;
    FOR i := 1 TO 10 DO
        total := total + i;
    WriteLn(total);

    isPositive := total > 0;
    WriteLn(isPositive);

    WriteLn
END.
