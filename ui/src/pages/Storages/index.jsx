import Typography from '@suid/material/Typography'
import Grid from '@suid/material/Grid'
import Stack from '@suid/material/Stack'
import Paper from '@suid/material/Paper'
import Table from '@suid/material/Table'
import TableBody from '@suid/material/TableBody'
import TableCell from '@suid/material/TableCell'
import TableContainer from '@suid/material/TableContainer'
import TableHead from '@suid/material/TableHead'
import TableRow from '@suid/material/TableRow'
import Button from '@suid/material/Button'
import AddIcon from '@suid/icons-material/Add'
import { Show, createSignal, mapArray, onMount } from 'solid-js'
import { useNavigate } from '@solidjs/router'

import API from '../../api'
import { convertSize } from '../../common/size_converter'

const Storages = () => {
	/**
	 * @type {[import("solid-js").Accessor<import("../../api").StorageWithInfo[]>, any]}
	 */
	const [storages, setStorages] = createSignal([])
	const navigate = useNavigate()

	onMount(async () => {
		const storagesSchema = await API.storages.listStorages()
		setStorages(storagesSchema.storages)
	})

	return (
		<Stack>
			<div class="page-header">
				<div>
					<h1>Storages</h1>
					<Typography color="text.secondary" sx={{ mt: 0.5 }}>
						Telegram-backed volumes for your files
					</Typography>
				</div>
				<Button
					onClick={() => navigate('/storages/register')}
					variant="contained"
					color="secondary"
					startIcon={<AddIcon />}
				>
					New storage
				</Button>
			</div>

			<Grid>
				<TableContainer component={Paper} class="surface-panel" elevation={0}>
					<Table sx={{ minWidth: 650 }}>
						<Show
							when={storages().length}
							fallback={
								<BoxEmpty message="No storages yet — create one in the UI (New storage), or set TELEGRAM_BOT_TOKEN, TELEGRAM_CHANNEL_ID, and STORAGE_NAME in .env for auto-setup." />
							}
						>
							<TableHead>
								<TableRow>
									<TableCell sx={{ fontWeight: 700 }}>Name</TableCell>
									<TableCell sx={{ fontWeight: 700 }}>Chat ID</TableCell>
									<TableCell sx={{ fontWeight: 700 }}>Size</TableCell>
									<TableCell sx={{ fontWeight: 700 }}>Files</TableCell>
								</TableRow>
							</TableHead>
							<TableBody>
								{mapArray(storages, (storage) => (
									<TableRow
										onClick={() => navigate(`/storages/${storage.id}/files`)}
										sx={{
											cursor: 'pointer',
											'&:last-child td, &:last-child th': { border: 0 },
										}}
									>
										<TableCell component="th" scope="row" sx={{ fontWeight: 600 }}>
											{storage.name}
										</TableCell>
										<TableCell>{storage.chat_id}</TableCell>
										<TableCell>{convertSize(storage.size)}</TableCell>
										<TableCell>{storage.files_amount}</TableCell>
									</TableRow>
								))}
							</TableBody>
						</Show>
					</Table>
				</TableContainer>
			</Grid>
		</Stack>
	)
}

const BoxEmpty = (props) => (
	<tbody>
		<tr>
			<td colSpan={4} style={{ padding: '48px 24px', 'text-align': 'center' }}>
				<Typography color="text.secondary">{props.message}</Typography>
			</td>
		</tr>
	</tbody>
)

export default Storages
