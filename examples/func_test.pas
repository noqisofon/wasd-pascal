PROGRAM FuncTest;

FUNCTION Double(x: INTEGER): INTEGER;
BEGIN
    Double := x * 2;
END;

PROCEDURE PrintGreeting(name: STRING[40]);
BEGIN
    WriteLn(name);
END;

VAR
    n: INTEGER;
BEGIN
    n := Double(21);
    WriteLn(n);
    PrintGreeting('Hello from a parameter!');
END.
