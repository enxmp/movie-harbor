const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
(async()=>{
 const browser=await chromium.launch({channel:'chrome',headless:true});
 const page=await browser.newPage({viewport:{width:1180,height:900}});const errors=[];page.on('pageerror',e=>errors.push(e.message));
 await page.addInitScript(()=>{window.dragListeners={};window.__TAURI_EVENT_PLUGIN_INTERNALS__={unregisterListener:()=>{}};window.__TAURI_INTERNALS__={metadata:{currentWebview:{label:"main"},currentWindow:{label:"main"}},transformCallback:fn=>fn,invoke:async(cmd,args)=>{
  if(cmd==='plugin:event|listen'){window.dragListeners[args.event]=args.handler;return 1;}
  if(cmd==='environment')return {ffmpeg:'ffmpeg',ffprobe:'ffprobe',nvenc:true,apple:false,platform:'windows',dualNvenc:true,splitSupported:true};
  if(cmd==='plugin:dialog|open')return args.options.directory?'D:\\Output':['D:\\Movies\\Sample movie.mkv'];
  if(cmd==='inspect')return {format:{duration:'120',size:'1200000000'},chapters:[],streams:[{index:0,codec_type:'video',codec_name:'h264',width:1920,height:1080,r_frame_rate:'24000/1001',avg_frame_rate:'24000/1001'},{index:1,codec_type:'audio',codec_name:'dts',channels:6,channel_layout:'5.1(side)',tags:{title:'Original English',language:'eng'}}]};
  if(cmd==='reveal_output'){window.revealed=args.path;return;}
  if(cmd==='start'){window.lastJob=args.jobs[0].job;window.lastParallel=args.parallel;window.lastId=args.jobs[0].id;return;}
  if(cmd==='status'&&!window.progressSent){window.progressSent=true;return {running:true,jobs:{[window.lastId]:{running:true,phase:'Encoding',seconds:60,speed:'2x',output:'',message:''}}};}
  if(cmd==='status')return {running:false,jobs:{[window.lastId]:{running:false,phase:'Complete',seconds:120,speed:'2x',output:'D:\\Output\\Sample movie - HEVC.mkv',message:''}}};
 }};});
 await page.goto('http://127.0.0.1:1420');await page.waitForFunction(()=>window.dragListeners['tauri://drag-drop']);
 await page.evaluate(()=>window.dragListeners['tauri://drag-enter']({payload:{paths:['D:\\Movies\\Sample movie.mkv'],position:{x:600,y:300}}}));
 await page.getByText('Drop movies to add to the queue',{exact:true}).waitFor();
 await page.evaluate(()=>window.dragListeners['tauri://drag-drop']({payload:{paths:['D:\\Movies\\Sample movie.mkv','D:\\Movies\\Sample movie.mkv'],position:{x:600,y:300}}}));
 await page.getByLabel('Output format for audio 1').waitFor();
 await page.evaluate(()=>window.dragListeners['tauri://drag-drop']({payload:{paths:['D:\\Movies\\Sample movie.mkv','D:\\Movies\\notes.txt'],position:{x:40,y:300}}}));
 await page.getByText('notes.txt: unsupported file type',{exact:true}).waitFor();
 if(await page.locator('.movie').count()!==1)throw Error('Duplicate drop added movie');
 await page.getByLabel('Dismiss error').click();

 await page.getByRole('button',{name:'Settings',exact:true}).click();
 if(await page.getByRole('switch',{name:'Keep original backups'}).isChecked())throw Error('Backup default must be off');
 if(!await page.getByLabel('Replace original').isChecked())throw Error('Replacement default must be on');
 await page.getByLabel('Replace original').uncheck();
 if(!await page.getByRole('switch',{name:/Recommended audio outputs/}).isChecked())throw Error('Recommendation default missing');
 if(!await page.getByRole('switch',{name:/Repair small audio/}).isChecked())throw Error('Repair default missing');
 if(await page.getByLabel('Simultaneous conversions').inputValue()!=='2')throw Error('Parallel default missing');
 if(!await page.getByRole('switch',{name:/NVIDIA split-frame/}).isChecked())throw Error('Split default missing');
 await page.getByLabel('Output format for audio 1').selectOption('aac');
 await page.getByRole('switch',{name:/Recommended audio outputs/}).check();
 if(await page.getByLabel('Output format for audio 1').inputValue()!=='eac3')throw Error('Recommendation not applied');
 await page.getByRole('switch',{name:/Recommended audio outputs/}).uncheck();
 if(await page.getByLabel('Output format for audio 1').inputValue()!=='aac')throw Error('Manual selection not restored');
 await page.getByText('Alongside original · choose folder').click();
 await page.getByRole('button',{name:'Settings',exact:true}).click();
 await page.screenshot({path:'ui-preview.png',fullPage:true});
 await page.getByText('Convert queue',{exact:true}).click();
 await page.waitForFunction(()=>document.querySelector('.movie')?.style.getPropertyValue('--movie-progress')==='50%');
 const bg=await page.locator('.movie').evaluate(e=>getComputedStyle(e).backgroundSize);if(bg!=='50% 2px')throw Error('Progress underline missing: '+bg);
 await page.getByText('D:\\Output\\Sample movie - HEVC.mkv',{exact:true}).waitFor();
 await page.locator('.movie').click({button:'right'});
 await page.getByRole('menuitem').click();
 if(await page.evaluate(()=>window.revealed)!=='D:\\Output\\Sample movie - HEVC.mkv')throw Error('Wrong revealed path');
 const job=await page.evaluate(()=>window.lastJob);
 if(job.keep_backup!==false)throw Error("Backup preference not submitted");
 if(!job.split_encode||await page.evaluate(()=>window.lastParallel)!==2)throw Error('Performance options not submitted');
 if(job.audio[0].mode!=='aac'||job.folder!=='D:\\Output'||job.replace)throw Error('Incorrect submitted settings');
 if(errors.length)throw Error(errors.join('\n'));
 console.log('UI smoke passed: add, select AAC, destination, queue completion; no React errors. IPC mocked.');await browser.close();
})().catch(e=>{console.error(e);process.exit(1)});
