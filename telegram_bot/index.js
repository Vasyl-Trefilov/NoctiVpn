require('dotenv').config();
const { Telegraf } = require('telegraf');
const { Pool } = require('pg');
const axios = require('axios');

const pool = new Pool({ connectionString: process.env.DATABASE_URL });
const CONTROL_PLANE_URL = process.env.CONTROL_PLANE_URL || 'http://control_plane:3000';

const bot = new Telegraf(process.env.TG_BOT_TOKEN);

async function ensureUser(tgId) {
  const client = await pool.connect();
  try {
    const existing = await client.query(
      'SELECT id, uuid FROM users WHERE tg_id = $1',
      [tgId]
    );
    if (existing.rows.length > 0) {
      return existing.rows[0];
    }
    const res = await axios.post(`${CONTROL_PLANE_URL}/api/v1/users`, {
      tg_id: Number(tgId),
    });
    const { id, uuid } = res.data;
    return { id, uuid };
  } finally {
    client.release();
  }
}

async function getUserStatus(tgId) {
  const { rows } = await pool.query(
    `SELECT u.uuid, s.expire_date, s.status
     FROM users u
     LEFT JOIN subscriptions s ON s.user_id = u.id
     WHERE u.tg_id = $1`,
    [tgId]
  );
  return rows[0] || null;
}

bot.start(async (ctx) => {
  try {
    const user = await ensureUser(ctx.from.id);
    await ctx.reply(
      `Welcome. You are registered.\nUUID: \`${user.uuid}\`\nUse /status for subscription info, /buy to extend.`,
      { parse_mode: 'Markdown' }
    );
  } catch (e) {
    console.error('start error:', e);
    await ctx.reply('Registration failed. Try again later.');
  }
});

bot.command('status', async (ctx) => {
  try {
    const row = await getUserStatus(ctx.from.id);
    if (!row) {
      await ctx.reply('You are not registered. Send /start first.');
      return;
    }
    const exp = row.expire_date
      ? new Date(row.expire_date).toISOString().slice(0, 10)
      : '—';
    const status = row.status || '—';
    await ctx.reply(
      `UUID: \`${row.uuid}\`\nSubscription: ${status}\nExpires: ${exp}`,
      { parse_mode: 'Markdown' }
    );
  } catch (e) {
    console.error('status error:', e);
    await ctx.reply('Could not load status.');
  }
});

bot.command('buy', async (ctx) => {
  await ctx.reply('Get a plan:', {
    reply_markup: {
      inline_keyboard: [
        [{ text: 'Payment Link', url: 'https://example.com/pay' }],
      ],
    },
  });
});

bot.launch().catch((e) => {
  console.error('Bot failed to start:', e);
  process.exit(1);
});

process.once('SIGINT', () => bot.stop('SIGINT'));
process.once('SIGTERM', () => bot.stop('SIGTERM'));
