FROM node:24.13.0-bookworm@sha256:1de022d8459f896fff2e7b865823699dc7a8d5567507e8b87b14a7442e07f206
RUN npm install --global @earendil-works/pi-coding-agent@0.87.1 \
    && npm cache clean --force
COPY access.ts /usr/local/lib/node_modules/@earendil-works/pi-coding-agent/ariel-access.ts
USER node
ENV HOME=/state PI_CODING_AGENT_DIR=/state/agent PI_OFFLINE=1 PI_SKIP_VERSION_CHECK=1 PI_TELEMETRY=0
ENTRYPOINT ["node", "/usr/local/lib/node_modules/@earendil-works/pi-coding-agent/dist/bundle/cli.js"]
