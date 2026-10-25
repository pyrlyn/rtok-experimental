-- T927: `memory`/`observe` rows booked the whole tool output as saved against the stored
-- narrative, but the agent still received that output, so every one of them was a saving that
-- never happened. New observations write no row; drop the ones already stored.
DELETE FROM measurements WHERE plugin = 'memory' AND kind = 'observe';
