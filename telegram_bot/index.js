require("dotenv").config();
const { Telegraf, Markup, session } = require("telegraf");
const { Pool } = require("pg");
const TonWeb = require("tonweb"); // Standard TON library
const QRCode = require("qrcode");

// --- Configuration ---
const pool = new Pool({ connectionString: process.env.DATABASE_URL });
const bot = new Telegraf(process.env.TG_BOT_TOKEN);
const ADMIN_WALLET = process.env.TON_WALLET_ADDRESS; // Your wallet receiving payments

// --- Database Helpers ---
const db = {
  async getUser(tgId) {
    const res = await pool.query("SELECT * FROM users WHERE tg_id = $1", [
      tgId,
    ]);
    return res.rows[0];
  },
  async createUser(tgId, username) {
    const res = await pool.query(
      "INSERT INTO users (tg_id, username) VALUES ($1, $2) ON CONFLICT (tg_id) DO UPDATE SET username = $2 RETURNING *",
      [tgId, username],
    );
    return res.rows[0];
  },
  async getActiveSub(userId) {
    const res = await pool.query(
      `SELECT s.*, t.name as plan_name, srv.domain, srv.ip_address, srv.public_key, srv.short_ids
       FROM subscriptions s
       JOIN tariffs t ON s.tariff_id = t.id
       JOIN servers srv ON s.server_id = srv.id
       WHERE s.user_id = $1 AND s.status = 'active' AND s.expire_date > now()`,
      [userId],
    );
    return res.rows[0];
  },
  async getTariffs() {
    const res = await pool.query("SELECT * FROM tariffs ORDER BY price ASC");
    return res.rows;
  },
  async findBestServer() {
    // Uses the view we created in SQL step
    const res = await pool.query(
      "SELECT id FROM view_server_load WHERE slots_available > 0 ORDER BY load_percentage ASC LIMIT 1",
    );
    return res.rows[0];
  },
  async createSubscription(userId, tariffId, serverId, durationDays = 30) {
    const client = await pool.connect();
    try {
      await client.query("BEGIN");

      // 1. Get Price
      const tRes = await client.query(
        "SELECT price FROM tariffs WHERE id = $1",
        [tariffId],
      );
      const price = tRes.rows[0].price;

      // 2. Deduct Balance
      const uRes = await client.query(
        "UPDATE users SET balance = balance - $1 WHERE id = $2 AND balance >= $1 RETURNING id",
        [price, userId],
      );

      if (uRes.rowCount === 0) throw new Error("Insufficient balance");

      // 3. Create Subscription
      // We generate a unique email for Xray logs: user_{tariff}_{uuid_segment}
      const xrayUuid = crypto.randomUUID();
      const email = `user_${tariffId}_${xrayUuid.substring(0, 8)}`;

      await client.query(
        `INSERT INTO subscriptions (user_id, server_id, tariff_id, xray_uuid, email, expire_date, status)
         VALUES ($1, $2, $3, $4, $5, now() + interval '${durationDays} days', 'active')`,
        [userId, serverId, tariffId, xrayUuid, email],
      );

      await client.query("COMMIT");
      return true;
    } catch (e) {
      await client.query("ROLLBACK");
      throw e;
    } finally {
      client.release();
    }
  },
};

// --- Bot Middleware ---
bot.use(session());
bot.use(async (ctx, next) => {
  if (ctx.from) {
    ctx.user = await db.createUser(ctx.from.id, ctx.from.username);
  }
  return next();
});

// --- Commands ---

bot.start(async (ctx) => {
  const txt =
    `👋 *Welcome to VPN Bot*\n\n` +
    `Your ID: \`${ctx.user.tg_id}\`\n` +
    `Balance: *${ctx.user.balance} RUB*\n\n` +
    `🚀 Fast, Secure, Reliable.`;

  await ctx.replyWithMarkdown(
    txt,
    Markup.inlineKeyboard([
      [
        Markup.button.callback("🛒 Buy VPN", "menu_tariffs"),
        Markup.button.callback("👤 My Profile", "menu_profile"),
      ],
      [Markup.button.callback("💎 Deposit TON", "menu_deposit")],
    ]),
  );
});

bot.action("menu_profile", async (ctx) => {
  const sub = await db.getActiveSub(ctx.user.id);

  let txt = `👤 *My Profile*\n\nBalance: *${ctx.user.balance} RUB*\n\n`;

  if (sub) {
    // Generate VLESS Link
    // format: vless://uuid@ip:443?security=reality&encryption=none&pbk=...&fp=chrome&type=tcp&flow=xtls-rprx-vision&sni=...&sid=...#Name
    const shortId = sub.short_ids[0] || "";
    const link = `vless://${sub.xray_uuid}@${sub.ip_address}:443?security=reality&encryption=none&pbk=${sub.public_key}&fp=chrome&type=tcp&flow=xtls-rprx-vision&sni=${sub.domain}&sid=${shortId}#VPN_${sub.plan_name}`;

    txt += `✅ *Active Subscription*\nPlan: ${sub.plan_name}\nExpires: ${new Date(sub.expire_date).toLocaleDateString()}\n\n👇 *Click to Copy Key:*`;
    await ctx.replyWithMarkdown(txt);
    await ctx.reply(`\`${link}\``, { parse_mode: "Markdown" });
  } else {
    txt += `❌ No active subscription.`;
    await ctx.editMessageText(txt, {
      parse_mode: "Markdown",
      ...Markup.inlineKeyboard([
        [Markup.button.callback("🛒 Buy Now", "menu_tariffs")],
      ]),
    });
  }
});

bot.action("menu_tariffs", async (ctx) => {
  const tariffs = await db.getTariffs();
  const buttons = tariffs.map((t) => [
    Markup.button.callback(
      `${t.name} - ${t.price} RUB (${t.speed_limit_mbps} Mbps)`,
      `buy_${t.id}`,
    ),
  ]);
  buttons.push([Markup.button.callback("🔙 Back", "start")]);

  await ctx.editMessageText("📋 *Select a Plan:*", {
    parse_mode: "Markdown",
    ...Markup.inlineKeyboard(buttons),
  });
});

// --- Buying Logic ---
bot.action(/^buy_(\d+)$/, async (ctx) => {
  const tariffId = ctx.match[1];
  const tariffs = await db.getTariffs();
  const plan = tariffs.find((t) => t.id == tariffId);

  if (parseFloat(ctx.user.balance) < parseFloat(plan.price)) {
    return ctx.reply(
      `⚠️ Insufficient balance. You need ${plan.price} RUB.\nCurrent: ${ctx.user.balance} RUB`,
      Markup.inlineKeyboard([
        [Markup.button.callback("💎 Deposit TON", "menu_deposit")],
      ]),
    );
  }

  // Find server
  const server = await db.findBestServer();
  if (!server)
    return ctx.reply("⚠️ No servers available right now. Please try later.");

  try {
    await db.createSubscription(ctx.user.id, plan.id, server.id);
    await ctx.reply(
      `✅ *Success!* You bought ${plan.name}.\nGo to Profile to get your key.`,
    );
  } catch (e) {
    console.error(e);
    await ctx.reply("Error processing purchase.");
  }
});

// --- TON Deposit Logic (Simplified) ---
bot.action("menu_deposit", async (ctx) => {
  // In production, you would generate a unique comment per transaction
  // For this example, we use the User TG ID as the comment
  const comment = `user-${ctx.user.tg_id}`;
  const amountTon = 1; // Example default
  const link = `ton://transfer/${ADMIN_WALLET}?amount=${amountTon * 1000000000}&text=${comment}`;

  const buffer = await QRCode.toBuffer(link);

  await ctx.replyWithPhoto(
    { source: buffer },
    {
      caption: `💎 *Top up Balance*\n\nSend TON to this address:\n\`${ADMIN_WALLET}\`\n\n⚠️ *CRITICAL:* You MUST include this comment:\n\`${comment}\`\n\n(Scanning the QR code sets this automatically)`,
      parse_mode: "Markdown",
    },
  );
});

// --- Start Bot ---
bot.launch();
process.once("SIGINT", () => bot.stop("SIGINT"));
process.once("SIGTERM", () => bot.stop("SIGTERM"));

// --- Mock Payment Watcher (Concept) ---
// In prod, run this every 30s to check your wallet for incoming transactions with comments
setInterval(async () => {
  // 1. Fetch transactions from TonCenter API
  // 2. Look for comment "user-12345"
  // 3. If found and not processed:
  //    pool.query('UPDATE users SET balance = balance + $1 WHERE tg_id = $2', [amount_in_rubles, 12345])
  //    bot.telegram.sendMessage(12345, "Balance updated!")
}, 60000);
