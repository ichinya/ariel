// Read-only metadata exporter; never returns credentials or invokes a prompt.
import { pathToFileURL } from 'node:url';
import { join } from 'node:path';
import { readFileSync } from 'node:fs';
const [packageRoot, profileRoot] = process.argv.slice(2);
const manifest = JSON.parse(readFileSync(join(packageRoot, 'package.json'), 'utf8'));
if (manifest.name !== '@earendil-works/pi-coding-agent' || manifest.version !== '0.87.1') throw Error('Pi 0.87.1 is required');
const { ModelRuntime } = await import(pathToFileURL(join(packageRoot, 'dist/index.js')));
const runtime = await ModelRuntime.create({authPath:join(profileRoot,'auth.json'),modelsPath:join(profileRoot,'models.json'),modelsStorePath:join(profileRoot,'catalog-cache'),allowModelNetwork:false,refreshOnCreate:false});
const auth=JSON.parse(readFileSync(join(profileRoot,'auth.json'),'utf8'));
const config=JSON.parse(readFileSync(join(profileRoot,'models.json'),'utf8'));
const configured=provider=>Boolean(auth[provider]?.type==='oauth'||auth[provider]?.type==='api_key'&&auth[provider]?.key||config.providers?.[provider]?.apiKey);
const models = runtime.getProviders().flatMap(provider => runtime.getModels(provider.id).map(m=>({provider:m.provider,id:m.id,name:m.name||m.id,api:m.api,baseUrl:m.baseUrl,reasoning:m.reasoning,input:m.input,contextWindow:m.contextWindow,maxTokens:m.maxTokens,cost:m.cost,configured:configured(provider.id)})));
process.stdout.write(JSON.stringify({version:manifest.version,models}));
