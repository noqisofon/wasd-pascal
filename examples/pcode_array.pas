PROGRAM PCodeArray;

VAR
    squares: ARRAY [1..5] OF INTEGER;
    flags: ARRAY [1..5] OF BOOLEAN;
    i, total: INTEGER;

BEGIN
    FOR i := 1 TO 5 DO
    BEGIN
        squares[i] := i * i;
        flags[i] := (i MOD 2) = 0
    END;

    total := 0;
    FOR i := 1 TO 5 DO
    BEGIN
        WriteLn(squares[i]);
        WriteLn(flags[i]);
        total := total + squares[i]
    END;

    WriteLn(total)
END.
