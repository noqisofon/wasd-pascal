PROGRAM PCodeProcedures;

VAR
    result: INTEGER;

FUNCTION Factorial(n: INTEGER): INTEGER;
BEGIN
    IF n <= 1 THEN
        Factorial := 1
    ELSE
        Factorial := n * Factorial(n - 1)
END;

PROCEDURE Increment(VAR value: INTEGER);
BEGIN
    value := value + 1
END;

BEGIN
    result := Factorial(5);
    Increment(result)
END.
