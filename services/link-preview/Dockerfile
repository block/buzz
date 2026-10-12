# Buzz link preview helper (not deployed). Build: docker build -t buzz-link-preview services/link-preview
FROM node:22-alpine
WORKDIR /app
COPY package.json package-lock.json* ./
RUN npm ci --omit=dev || npm install --omit=dev
COPY lib.mjs fetcher.mjs server.mjs ./
USER node
ENV PORT=8787
EXPOSE 8787
HEALTHCHECK CMD wget -qO- http://127.0.0.1:8787/healthz || exit 1
CMD ["node", "server.mjs"]
