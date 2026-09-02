PROGRAM PCodeMinimal;

VAR
    total: INTEGER;
    i: INTEGER;

BEGIN
    total := 0;
    FOR i := 1 TO 10 DO
        total := total + i;
    IF total > 0 THEN
        total := total
    ELSE
        total := 0
END.
