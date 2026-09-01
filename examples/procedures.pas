PROGRAM Procedures;

VAR
    result: INTEGER;

FUNCTION Factorial(n: INTEGER): INTEGER;
BEGIN
    IF n <= 1 THEN
        Factorial := 1
    ELSE
        Factorial := n * Factorial(n - 1)
END;

PROCEDURE PrintResult(tag: INTEGER; value: INTEGER);
BEGIN
    WriteLn(tag);
    WriteLn(value)
END;

BEGIN
    result := Factorial(5);
    PrintResult(5, result)
END.
