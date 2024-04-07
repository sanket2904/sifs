<script>
// @ts-nocheck

	import { invoke } from '@tauri-apps/api/tauri'


	let files = []
    // @ts-ignore
    async function search(query_str) {
        let res = await invoke('search_files', { query: query_str })
		console.log(res)
		files = res
    }



    
	
</script>

<div class="grid">
	<div>
		<h1>Welcome to SIFS</h1>
	</div>
	<div>
		<!-- create a search bar here  -->
		<input type="text" placeholder="Search" on:keydown={(e) => {
            if (e.key === 'Enter') {
                console.log('Enter key pressed')
                if (e.target.value) {
                    search(e.target.value)
                }
            }
        }} />
	</div>

	<div>
		<!-- table to show the search result -->

		<table>
			<thead>
				<tr>
					<th> File Name </th>
					<th> File location </th>
					<!-- <th> File Size </th> -->
					<!-- <th> File Type </th> -->
				</tr>
			</thead>

			<tbody>
				{#each files as file }
				  <tr on:dblclick={ async (e) => {
				  	//remove \ from the path
					let check = file.location.replace(/\\/g, '')
					console.log(file.location)
					let a = await invoke('open_file_or_folder', { path: `${check}\\${file.name}` })
				  }}>
					<td>{file.name}</td>
					<td>{file.location}</td>
					<!-- <td>{file.size}</td> -->
					<!-- <td>{file.type}</td> -->
				  </tr>
				{/each}
			  </tbody>
		</table>
	</div>
</div>

<style>
	/* all text color white  */

	h1 {
		color: #fefefe;
	}

	.grid {
		/* display: grid; */
		/* grid-template-rows: 1fr 1fr 6fr; */
		/* grid-gap: 20px; */
		margin: 20px;
	}

	/* search bar style  */

	input[type='text'] {
		width: 100%;
		padding: 10px 10px;
		margin: 8px 0;
		box-sizing: border-box;
		/* grey border  */
		border: 1px solid #27272a;
		border-radius: 4px;
		/* background color one shade lighter than #111 */
		background-color: #000;
		color: #fefefe;
	}

	/* table style  */

	table {
		width: 100%;
		border-collapse: collapse;
	}

	th {
		color: #a1a1aa;
		padding: 8px;
		text-align: left;
		font-size: 14px;
		padding: 10px;
	}
	thead {
		border: 1px solid #27272a;
		border-radius: 4px;
	}

	tbody {
		border: 1px solid #27272a;
		border-radius: 4px;
	}

	tr {
		border: 1px solid #27272a;
	}
	td {
		/* background-color: #27272a; */
		color: #fefefe;
		padding: 8px;
		text-align: left;
		/* border: 1px solid #27272a ; */
		font-size: 14px;
		padding: 10px;
	}
</style>
