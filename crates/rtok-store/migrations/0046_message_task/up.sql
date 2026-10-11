-- T441.13.1: the task (`A12`) a message is about, so a thread on one task reads apart from the
-- rest. NULL for a message sent without one and for rows written before this column existed.
ALTER TABLE messages ADD COLUMN task TEXT;
