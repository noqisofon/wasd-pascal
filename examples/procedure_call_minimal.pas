PROGRAM ProcTest;

PROCEDURE Greet;
BEGIN
    WriteLn('Hello from a procedure!');
END;

BEGIN
    Greet;
    WriteLn('Back in main.');
END.
