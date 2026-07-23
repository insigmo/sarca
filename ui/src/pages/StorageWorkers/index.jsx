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
import IconButton from '@suid/material/IconButton'
import DeleteIcon from '@suid/icons-material/Delete'
import AddIcon from '@suid/icons-material/Add'
import { Show, createSignal, mapArray, onMount } from 'solid-js'
import { useNavigate } from '@solidjs/router'

import API from '../../api'
import ActionConfirmDialog from '../../components/ActionConfirmDialog'

const StorageWorkers = () => {
	/**
	 * @type {[import("solid-js").Accessor<import("../../api").StorageWorker[]>, any]}
	 */
	const [storageWorkers, setStorageWorkers] = createSignal([])
	const [pendingDelete, setPendingDelete] = createSignal(null)
	const navigate = useNavigate()

	const refresh = async () => {
		const storageWorkers = await API.storageWorkers.listStorageWorkers()
		setStorageWorkers(storageWorkers)
	}

	onMount(refresh)

	const confirmDelete = async () => {
		const sw = pendingDelete()
		setPendingDelete(null)
		if (!sw) return
		await API.storageWorkers.deleteStorageWorker(sw.id)
		await refresh()
	}

	return (
		<Stack>
			<div class="page-header">
				<div>
					<h1>Storage workers</h1>
					<Typography color="text.secondary" sx={{ mt: 0.5 }}>
						Telegram bots that upload and download chunks
					</Typography>
				</div>
				<Button
					onClick={() => navigate('/storage_workers/register')}
					variant="contained"
					color="secondary"
					startIcon={<AddIcon />}
				>
					New worker
				</Button>
			</div>

			<Grid>
				<TableContainer component={Paper} class="surface-panel" elevation={0}>
					<Table sx={{ minWidth: 650 }}>
						<Show
							when={storageWorkers().length}
							fallback={
								<tbody>
									<tr>
										<td
											colSpan={4}
											style={{
												padding: '48px 24px',
												'text-align': 'center',
											}}
										>
											<Typography color="text.secondary">
												No storage workers yet — register a bot token in the UI
												(New worker), or set TELEGRAM_BOT_TOKEN,
												TELEGRAM_CHANNEL_ID, and STORAGE_NAME in .env for
												auto-setup.
											</Typography>
										</td>
									</tr>
								</tbody>
							}
						>
							<TableHead>
								<TableRow>
									<TableCell sx={{ fontWeight: 700 }}>Name</TableCell>
									<TableCell sx={{ fontWeight: 700 }}>Storage</TableCell>
									<TableCell sx={{ fontWeight: 700 }}>Token</TableCell>
									<TableCell align="right" sx={{ fontWeight: 700 }}>
										Actions
									</TableCell>
								</TableRow>
							</TableHead>
							<TableBody>
								{mapArray(storageWorkers, (sw) => (
									<TableRow
										sx={{ '&:last-child td, &:last-child th': { border: 0 } }}
									>
										<TableCell component="th" scope="row" sx={{ fontWeight: 600 }}>
											{sw.name}
										</TableCell>
										<TableCell>{sw.storage_id}</TableCell>
										<TableCell
											sx={{
												maxWidth: 280,
												overflow: 'hidden',
												textOverflow: 'ellipsis',
												whiteSpace: 'nowrap',
												fontFamily: 'ui-monospace, monospace',
												fontSize: '0.85rem',
											}}
										>
											{sw.token}
										</TableCell>
										<TableCell align="right">
											<IconButton
												aria-label="delete"
												onClick={() => setPendingDelete(sw)}
												sx={{ color: 'error.main' }}
											>
												<DeleteIcon />
											</IconButton>
										</TableCell>
									</TableRow>
								))}
							</TableBody>
						</Show>
					</Table>
				</TableContainer>
			</Grid>

			<ActionConfirmDialog
				action="Delete"
				entity="storage worker"
				actionDescription={`delete storage worker ${pendingDelete()?.name || ''}`}
				isOpened={Boolean(pendingDelete())}
				onConfirm={confirmDelete}
				onCancel={() => setPendingDelete(null)}
			/>
		</Stack>
	)
}

export default StorageWorkers
