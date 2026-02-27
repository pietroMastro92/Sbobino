import os

with open("/Users/pietromastro/Documents/sbobino_tauri/icon_source.b64", "r") as f:
    b64_data = f.read().replace('\n', '')

html_content = f"""<!DOCTYPE html>
<html>
<head><meta charset="utf-8"></head>
<body>
    <img id="source" src="data:image/png;base64,{b64_data}" style="display:none;" />
    <canvas id="canvas"></canvas>
    <div id="output">PROCESSING</div>
    <script>
        const img = document.getElementById('source');
        const canvas = document.getElementById('canvas');
        const ctx = canvas.getContext('2d', {{ willReadFrequently: true }});
        const outputBtn = document.getElementById('output');
        
        img.onload = function() {{
            setTimeout(processImage, 100);
        }};
        if (img.complete) {{
            processImage();
        }}

        function processImage() {{
            try {{
                const width = img.naturalWidth;
                const height = img.naturalHeight;
                // AI squircle is roughly 82% of the center width.
                const cropSize = width * 0.81;
                canvas.width = cropSize;
                canvas.height = cropSize;
                
                const radius = cropSize * 0.225;
                ctx.beginPath();
                ctx.moveTo(radius, 0);
                ctx.lineTo(cropSize - radius, 0);
                ctx.quadraticCurveTo(cropSize, 0, cropSize, radius);
                ctx.lineTo(cropSize, cropSize - radius);
                ctx.quadraticCurveTo(cropSize, cropSize, cropSize - radius, cropSize);
                ctx.lineTo(radius, cropSize);
                ctx.quadraticCurveTo(0, cropSize, 0, cropSize - radius);
                ctx.lineTo(0, radius);
                ctx.quadraticCurveTo(0, 0, radius, 0);
                ctx.closePath();
                ctx.clip(); 
                
                const dx = (cropSize - width) / 2;
                const dy = (cropSize - height) / 2;
                ctx.drawImage(img, dx, dy, width, height);
                
                const dataURL = canvas.toDataURL('image/png');
                const b64 = dataURL.split(',')[1];
                
                // Signal done, store b64 so the python script can extract it via browser_subagent
                const pre = document.createElement('textarea');
                pre.id = 'b64';
                pre.value = b64;
                document.body.appendChild(pre);
                outputBtn.innerHTML = 'DONE';
            }} catch (e) {{
                outputBtn.innerHTML = 'ERROR: ' + e;
            }}
        }}
    </script>
</body>
</html>"""

with open("/Users/pietromastro/Documents/sbobino_tauri/crop.html", "w") as f:
    f.write(html_content)
