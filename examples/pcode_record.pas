PROGRAM RecordTest;

TYPE
    Character = RECORD
        hp: INTEGER;
        alive: BOOLEAN;
    END;

VAR
    hero: Character;

BEGIN
    hero.hp := 100;
    hero.alive := TRUE;

    WriteLn(hero.hp);

    hero.hp := hero.hp - 30;
    WriteLn(hero.hp)
END.
