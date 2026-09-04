PROGRAM MultiArgTest;
TYPE
    Character = RECORD
        hp: INTEGER;
        alive: BOOLEAN;
    END;

FUNCTION Add(a: INTEGER; b: INTEGER; c: INTEGER): INTEGER;
BEGIN
    Add := a + b + c;
END;

PROCEDURE Damage(VAR ch: Character; amount: INTEGER);
BEGIN
    ch.hp := ch.hp - amount;
    IF ch.hp <= 0 THEN
        ch.alive := FALSE;
END;

VAR
    hero: Character;
    total: INTEGER;
BEGIN
    total := Add(10, 20, 30);
    WriteLn(total);

    hero.hp := 50;
    hero.alive := TRUE;
    Damage(hero, 30);
    WriteLn(hero.hp);
END.
