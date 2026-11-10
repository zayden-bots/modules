INSERT INTO guilds (id)
    VALUES (1), (2), (3), (4), (5), (6);

INSERT INTO deadlock_patch_announce (guild_id, channel_id)
    VALUES (1, 101), (2, 102), (3, 103), (4, 104), (5, 105), (6, 106);

-- Zayden is application 1000; guild 6 is served by the custom bot 2000.
INSERT INTO guild_presence (guild_id, application_id, left_at)
    VALUES
        (1, 1000, NULL),
        (2, 1000, NULL),
        (3, 1000, NULL),
        -- Zayden left guild 5.
        (5, 1000, now()),
        (6, 1000, NULL),
        -- The custom bot is also in guild 1, which it does not serve.
        (1, 2000, NULL),
        (6, 2000, NULL);

INSERT INTO guild_app_modules (guild_id, application_id, module, enabled)
    VALUES
        -- Switched on explicitly.
        (2, 1000, 'deadlock', TRUE),
        -- Switched off.
        (3, 1000, 'deadlock', FALSE),
        -- Another application switched it off in guild 1.
        (1, 3000, 'deadlock', FALSE);

INSERT INTO applications (application_id, slot, kind, bot_user_id, name)
    VALUES (1000, 'zayden', 'default', 1001, 'Zayden');

INSERT INTO applications (application_id, slot, kind, bot_user_id, name, public_key, token_ct, token_nonce, key_id, registered_by)
    VALUES (2000, 'zayden', 'custom', 2001, 'Custom', 'key', '\x00'::bytea, '\x00'::bytea, 1, 1);

INSERT INTO application_guilds (guild_id, slot, application_id, attached_by)
    VALUES (6, 'zayden', 2000, 1);

INSERT INTO entitlements (provider, external_id, scope_type, scope_id, tier)
    VALUES ('test', 'guild-6', 'guild', 6, 'ultra');
