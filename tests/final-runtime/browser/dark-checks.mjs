import assert from 'node:assert/strict'
export async function verifyDark({page,productID,capture,save}){
 const layouts=[]
 async function geometry(label,controls=[]){
  const layout=await page.evaluate(()=>({viewport:innerWidth,width:document.documentElement.scrollWidth,client:document.documentElement.clientWidth,dark:document.documentElement.classList.contains('dark'),body_foreground:getComputedStyle(document.body).color,body_background:getComputedStyle(document.body).backgroundColor}))
  const targets=[]
  for(const [name,control] of controls){
   await control.scrollIntoViewIfNeeded()
   const box=await control.boundingBox()
   targets.push({name,box})
  }
  const observation={label,...layout,controls:targets}
  layouts.push(observation)
  await save('dark-layout.json',layouts)
  assert.equal(layout.dark,true);assert.ok(layout.width<=layout.client+1)
  for(const {box} of targets)assert.ok(box&&box.width>0&&box.height>0&&box.x>=-1&&box.x+box.width<=layout.viewport+1)
 }
 await page.setViewportSize({width:1440,height:1000})
 await page.getByRole('heading',{name:'Seller center',exact:true}).waitFor({state:'visible'})
 const product=page.locator('[data-store-product-id='+JSON.stringify(String(productID))+']')
 await product.waitFor({state:'visible'})
 assert.ok((await product.getByRole('heading',{level:2}).innerText()).trim())
 await page.getByRole('button',{name:'Theme options',exact:true}).click()
 await page.getByRole('menuitem',{name:'Dark',exact:true}).click()
 await page.waitForFunction(()=>document.documentElement.classList.contains('dark'))
 const connect=page.getByRole('button',{name:'Connect Extore',exact:true})
 const create=page.getByRole('button',{name:'New product',exact:true})
 const edit=product.getByRole('button',{name:'Edit',exact:true}).first()
 await geometry('desktop-manage',[['Connect Extore',connect],['New product',create],['Imported product Edit',edit]])
 await page.evaluate(()=>window.scrollTo(0,0));await capture('dark-imported-products-desktop')
 await edit.click()
 const dialog=page.getByRole('dialog',{name:'Edit product',exact:true})
 await dialog.waitFor({state:'visible'})
 await geometry('desktop-imported-product-dialog')
 await capture('dark-imported-product-dialog-desktop')
 await page.keyboard.press('Escape');await dialog.waitFor({state:'hidden'})
 await page.setViewportSize({width:390,height:844})
 await geometry('mobile-manage',[['Connect Extore',connect],['New product',create],['Imported product Edit',edit]])
 await page.evaluate(()=>window.scrollTo(0,0));await capture('dark-imported-products-mobile')
 await edit.click();await dialog.waitFor({state:'visible'})
 const box=await dialog.boundingBox()
 await save('dark-mobile-dialog-bounds.json',{box,viewport:390})
 assert.ok(box&&box.x>=-1&&box.x+box.width<=391)
 await geometry('mobile-imported-product-dialog')
 await capture('dark-imported-product-dialog-mobile')
 await page.keyboard.press('Escape');await dialog.waitFor({state:'hidden'})
 await page.reload()
 await page.getByRole('heading',{name:'Seller center',exact:true}).waitFor({state:'visible'})
 await product.waitFor({state:'visible'})
 await geometry('mobile-dark-persisted-after-reload')
 return {existing_theme_control_and_persistence:'PASS',imported_product_manage_and_dialog_390px_no_horizontal_overflow:'PASS'}
}
