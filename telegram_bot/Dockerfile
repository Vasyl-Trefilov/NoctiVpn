FROM node:20-alpine
WORKDIR /app
COPY telegram_bot/package.json telegram_bot/package-lock.json* ./
RUN npm ci --omit=dev 2>/dev/null || npm install --omit=optional
COPY telegram_bot/*.js ./
CMD ["node", "index.js"]
