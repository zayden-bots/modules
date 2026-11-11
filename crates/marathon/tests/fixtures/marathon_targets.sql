INSERT INTO guilds (id)
VALUES (1), (2);

INSERT INTO marathon_announce (guild_id, channel_id)
VALUES (1, 101), (2, 102);

INSERT INTO applications (application_id, slot, kind, bot_user_id, name)
VALUES (1000, 'zayden', 'default', 1001, 'Zayden');

-- Zayden is in guild 1 only.
INSERT INTO guild_presence (guild_id, application_id, left_at)
VALUES (1, 1000, NULL);
