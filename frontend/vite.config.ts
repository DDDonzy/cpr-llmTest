import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'
export default defineConfig({
 plugins:[vue(),{name:'cpr-classic-page',apply:'build',transformIndexHtml:{order:'post',handler:(html:string)=>html.replace(/ type="module"/g,'').replace(/ crossorigin/g,'').replace(/<script src=/g,'<script defer src=')}}],
 base:'./',build:{outDir:'../web',emptyOutDir:true,cssCodeSplit:false,rollupOptions:{output:{format:'iife',name:'CprAccountDialogue',entryFileNames:'app.js',assetFileNames:asset=>asset.names?.some(n=>n.endsWith('.css'))?'app.css':'[name][extname]',codeSplitting:false}}}
})
